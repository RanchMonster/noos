use core::slice;

use alloc::vec::Vec;
use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, MappedPageTable, Mapper, OffsetPageTable, Page, PageSize,
        PageTable, PageTableFlags, PhysFrame, Size1GiB, Size2MiB, Size4KiB, mapper,
    },
};

use crate::serial_println;

/// Initialize a new OffsetPageTable.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    unsafe {
        let level_4_table = active_level_4_table(physical_memory_offset);
        OffsetPageTable::new(level_4_table, physical_memory_offset)
    }
}

/// Returns a mutable reference to the active level 4 table.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

/// A FrameAllocator that always returns `None`.
pub struct EmptyFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for EmptyFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        None
    }
}

/// A FrameAllocator that returns usable frames from the bootloader's memory map.
pub struct BootInfoFrameAllocator {
    memory_map: &'static MemoryMap,
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Create a FrameAllocator from the passed memory map.
    ///
    /// This function is unsafe because the caller must guarantee that the passed
    /// memory map is valid. The main requirement is that all frames that are marked
    /// as `USABLE` in it are really unused.
    pub unsafe fn init(memory_map: &'static MemoryMap) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
        }
    }

    /// Returns an iterator over the usable frames specified in the memory map.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.region_type == MemoryRegionType::Usable);
        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.range.start_addr()..r.range.end_addr());
        // transform to an iterator of frame start addresses
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        // create `PhysFrame` types from the start addresses
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
    fn max_physical_address(&self) -> u64 {
        self.memory_map
            .iter()
            .map(|r| r.range.end_addr())
            .max()
            .expect("no memory region?")
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
const BITMAP_START: usize = 0xffff_8000_1000_0000;
pub struct BitMapFrameAllocator {
    bitmap: &'static mut [u64],
    next_search_word: usize,
}
impl BitMapFrameAllocator {
    pub fn new(
        mapper: &mut impl Mapper<Size4KiB>,
        mut boot_info_frame_allocator: BootInfoFrameAllocator,
    ) -> Self {
        let max_addr = boot_info_frame_allocator.max_physical_address() as usize;

        // Each 4KiB frame needs 1 bit in the bitmap.
        // total_frames = max physical address / page size, rounded up
        let total_frames = max_addr.div_ceil(4096);

        // Pack bits into u64 words (64 frames per word)
        let bitmap_words = total_frames.div_ceil(64);

        // Number of 4KiB pages to physically store the bitmap array
        let bitmap_pages = (bitmap_words * 8).div_ceil(4096);
        for i in 0..bitmap_pages {
            let page = Page::containing_address(VirtAddr::new((BITMAP_START + i * 4096) as u64));
            let frame = boot_info_frame_allocator
                .allocate_frame()
                .expect("no usable frame available for bitmap");
            unsafe {
                mapper
                    .map_to(
                        page,
                        frame,
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                        &mut boot_info_frame_allocator,
                    )
                    .expect("failed to map bitmap")
                    .flush();
            }
        }

        // View the mapped bitmap memory as a mutable slice of u64 words
        let bitmap = unsafe { slice::from_raw_parts_mut(BITMAP_START as *mut u64, bitmap_words) };

        // Start with all frames marked as used (all bits = 1)
        for word in bitmap.iter_mut() {
            *word = u64::MAX;
        }

        let total_used_frames = boot_info_frame_allocator.next;

        // The frames just allocated for the bitmap pages are already marked used.
        // Mark the remaining usable frames as free by clearing their bits.
        let usable_frames = boot_info_frame_allocator
            .usable_frames()
            .skip(total_used_frames);

        for frame in usable_frames {
            let frame_index = frame.start_address().as_u64() / 4096;
            let word_index = (frame_index / 64) as usize;
            let bit_index = frame_index % 64;
            bitmap[word_index] &= !(1 << bit_index);
        }

        BitMapFrameAllocator {
            bitmap,
            next_search_word: total_used_frames / 64,
        }
    }

    /// Helper: Checks if a sequential range of bits in the bitmap is entirely 0 (free)
    fn is_region_free(&self, start_frame: usize, count: usize) -> bool {
        let end_frame = start_frame + count;

        for frame in start_frame..end_frame {
            let word_idx = frame / 64;
            let bit_idx = frame % 64;

            // Guard: Array bounds check safety
            assert!(word_idx < self.bitmap.len());

            // If any bit is set (1), the block is not free
            let is_allocated = (self.bitmap[word_idx] & (1 << bit_idx)) != 0;
            if is_allocated {
                return false;
            }
        }

        true
    }
    fn is_region_used(&self, start_frame: usize, count: usize) -> bool {
        !self.is_region_free(start_frame, count)
    }

    /// Helper: Flips a sequential range of bits from 0 to 1 (allocated)
    fn mark_region_allocated(&mut self, start_frame: usize, count: usize) {
        debug_assert!(
            self.is_region_free(start_frame, count),
            "Allocated region overlaps with free region"
        ); // if I use this right, this should never happen (but just in case it doing a assertion
        // in debug mode)
        let end_frame = start_frame + count;

        for frame in start_frame..end_frame {
            let word_idx = frame / 64;
            let bit_idx = frame % 64;

            // Use bitwise OR to set the specific target bit to 1
            self.bitmap[word_idx] |= 1 << bit_idx;
        }
    }
    fn mark_region_free(&mut self, start_frame: usize, count: usize) {
        debug_assert!(
            self.is_region_used(start_frame, count),
            "Free region overlaps with allocated region"
        ); // if I use this right, this should never happen (but just in case it doing a assertion
        // in debug mode)
        let end_frame = start_frame + count;
        for frame in start_frame..end_frame {
            let word_idx = frame / 64;
            let bit_idx = frame % 64;

            // Use bitwise AND to set the specific target bit to 0
            self.bitmap[word_idx] &= !(1 << bit_idx);
        }
    }
}
unsafe impl FrameAllocator<Size4KiB> for BitMapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        for word_idx in self.next_search_word..self.bitmap.len() {
            let word = self.bitmap[word_idx];
            // u64::MAX means all 64 frames in this word are used (all bits = 1)
            if word == u64::MAX {
                continue;
            }
            // Free frames have bit = 0. trailing_zeros finds the first 0 bit on the right.
            let bit = (!word).trailing_zeros();
            let bit = bit as usize;

            // Mark this frame as used
            self.bitmap[word_idx] |= 1 << bit;

            // Frame number = word_index * 64 (frames per word) + bit position
            // Physical address = frame_number * 4096 (bytes per frame)
            let frame_number = word_idx * 64 + bit;
            let addr = PhysAddr::new((frame_number as u64) * 4096);

            // Advance search hint so next call doesn't re-scan known-used words
            self.next_search_word = word_idx;
            serial_println!("allocating frame: {}", addr.as_u64());
            return Some(PhysFrame::containing_address(addr));
        }
        None
    }
}
impl FrameDeallocator<Size4KiB> for BitMapFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame) {
        let frame_index = frame.start_address().as_u64() / 4096;
        let word_index = (frame_index / 64) as usize;
        let bit_index = frame_index % 64;
        self.bitmap[word_index] &= !(1 << bit_index);
        // Optionally rewind next_search_word so this frame gets reused
        self.next_search_word = self.next_search_word.min(word_index);
    }
}
unsafe impl FrameAllocator<Size2MiB> for BitMapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size2MiB>> {
        for word_idx in self.next_search_word..self.bitmap.len() {
            let word = self.bitmap[word_idx];

            // Guard: Skip fully allocated words immediately
            if word == u64::MAX {
                continue;
            }

            // Find the first free bit index (0 to 63)
            let bit = (!word).trailing_zeros() as usize;
            let start_frame = word_idx * 64 + bit;

            // Guard: 2 MiB frames must start at a 512-aligned boundary
            if start_frame % 512 != 0 {
                continue;
            }

            // Guard: Ensure all 512 bits starting here are completely free
            let out_of_bounds = word_idx >= self.bitmap.len();
            if out_of_bounds || !self.is_region_free(start_frame, 512) {
                continue;
            }

            // If we passed all guards, perform allocation and return
            self.mark_region_allocated(start_frame, 512);
            self.next_search_word = (start_frame + 512) / 64;

            let phys_addr = PhysAddr::new((start_frame * 4096) as u64);
            return Some(PhysFrame::containing_address(phys_addr));
        }

        None
    }
}
impl FrameDeallocator<Size2MiB> for BitMapFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size2MiB>) {
        let frame_index = (frame.start_address().as_u64() / 4096) as usize;
        self.mark_region_free(frame_index, 512);
    }
}
unsafe impl FrameAllocator<Size1GiB> for BitMapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size1GiB>> {
        unimplemented!("1GiB allocation not implemented")
    }
}

impl FrameDeallocator<Size1GiB> for BitMapFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size1GiB>) {
        unimplemented!("1GiB deallocation not implemented")
    }
}
