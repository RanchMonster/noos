use crate::println;
use alloc::slice;
use bootloader_api::info::{MemoryRegion, MemoryRegionKind, MemoryRegions};
use x86_64::{
    PhysAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, Mapper, PageSize, PageTableFlags, PhysFrame, Size1GiB,
        Size2MiB, Size4KiB,
    },
};
const PAGE_SIZE: usize = Size4KiB::SIZE as usize;

// rust-analyzer is being dumb about the formatting of these constants,
// it might be a bug I don't feel like tracking it down right this instant.
#[allow(non_upper_case_globals)]
const NUMBER_OF_4KiB_PAGES_IN_2MiB: usize = (Size2MiB::SIZE / Size4KiB::SIZE) as usize;
#[allow(non_upper_case_globals)]
const NUMBER_OF_4KiB_PAGES_IN_1GiB: usize = (Size1GiB::SIZE / Size4KiB::SIZE) as usize;

pub struct BitMapFrameAllocator {
    bitmap: &'static mut [u64],
    next_search_word: usize,
}

impl BitMapFrameAllocator {
    pub const fn new() -> Self {
        Self {
            bitmap: &mut [],
            next_search_word: 0,
        }
    }

    pub fn init(&mut self, memory_map: &'static [MemoryRegion], physical_offset: usize) {
        assert!(physical_offset != 0, "Physical offset must be non-zero");

        let highest_address = memory_map
            .iter()
            .map(|region| region.end)
            .max()
            .expect("No memory region?");
        let total_frames = (highest_address / PAGE_SIZE as u64) as usize;
        let bitmap_words = total_frames.div_ceil(64);
        let bitmap_bytes = bitmap_words * core::mem::size_of::<u64>();

        let bitmap_fits_in_region = memory_map
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .find(|region| (region.end - region.start) as usize >= bitmap_bytes)
            .expect("No memory region found for the bit map");

        let bitmap_start = bitmap_fits_in_region.start as usize;
        assert!(
            bitmap_start + bitmap_bytes <= bitmap_fits_in_region.end as usize,
            "The bit map doesn't fit in the memory region"
        );

        self.bitmap = unsafe {
            slice::from_raw_parts_mut(
                (bitmap_start + physical_offset) as *mut u64,
                bitmap_words,
            )
        };

        // Reserve every frame by default so that non-usable memory (and any
        // buddy that falls outside a usable region) is never treated as free.
        self.bitmap.fill(u64::MAX);

        for region in memory_map
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
        {
            self.mark_region_free(
                (region.start / PAGE_SIZE as u64) as usize,
                ((region.end - region.start) / PAGE_SIZE as u64) as usize,
            );
        }

        // The bitmap describes itself, so re-reserve the frames it occupies.
        self.mark_region_allocated(
            (bitmap_start / PAGE_SIZE) as usize,
            bitmap_bytes.div_ceil(PAGE_SIZE),
        );

        self.next_search_word = 0;
    }

    /// Helper: Checks if a sequential range of bits in the bitmap is entirely 0 (free)
    pub fn is_region_free(&self, start_frame: usize, count: usize) -> bool {
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
    pub fn is_region_used(&self, start_frame: usize, count: usize) -> bool {
        !self.is_region_free(start_frame, count)
    }

    /// Helper: Flips a sequential range of bits from 0 to 1 (allocated)
    /// # Arguments
    /// * `start_frame` - The start frame number
    /// * `count` - The number of frames to flip
    pub fn mark_region_allocated(&mut self, start_frame: usize, count: usize) {
        assert!(
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

    /// Helper: Flips a sequential range of bits from 1 to 0 (free)
    /// # Arguments
    /// * `start_frame` - The start frame number
    /// * `count` - The number of frames to flip
    pub fn mark_region_free(&mut self, start_frame: usize, count: usize) {
        assert!(
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
            println!("allocating frame: {}", addr.as_u64());
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

            // Guard: 2 MiB frames must start at a NUMBER_OF_4KiB_PAGES_IN_2MiB-aligned boundary
            if !start_frame.is_multiple_of(NUMBER_OF_4KiB_PAGES_IN_2MiB) {
                continue;
            }

            // Guard: Ensure all NUMBER_OF_4KiB_PAGES_IN_2MiB bits starting here are completely free
            let out_of_bounds = word_idx >= self.bitmap.len();
            if out_of_bounds || !self.is_region_free(start_frame, NUMBER_OF_4KiB_PAGES_IN_2MiB) {
                continue;
            }

            // If we passed all guards, perform allocation and return
            self.mark_region_allocated(start_frame, NUMBER_OF_4KiB_PAGES_IN_2MiB);
            self.next_search_word = (start_frame + NUMBER_OF_4KiB_PAGES_IN_2MiB) / 64;

            let phys_addr = PhysAddr::new((start_frame * 4096) as u64);
            return Some(PhysFrame::containing_address(phys_addr));
        }

        None
    }
}

impl FrameDeallocator<Size2MiB> for BitMapFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size2MiB>) {
        let frame_index = (frame.start_address().as_u64() / Size4KiB::SIZE) as usize;
        self.mark_region_free(frame_index, NUMBER_OF_4KiB_PAGES_IN_2MiB);
        self.next_search_word = self.next_search_word.min(frame_index / 64);
    }
}
unsafe impl FrameAllocator<Size1GiB> for BitMapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size1GiB>> {
        for word_idx in self.next_search_word..self.bitmap.len() {
            let word = self.bitmap[word_idx];

            // Guard: Skip fully allocated words immediately
            if word == u64::MAX {
                continue;
            }

            // Find the first free bit index (0 to 63)
            let bit = (!word).trailing_zeros() as usize;
            let start_frame = word_idx * 64 + bit;

            // Guard: 2 MiB frames must start at a NUMBER_OF_4KiB_PAGES_IN_1GiB-aligned boundary
            if !start_frame.is_multiple_of(NUMBER_OF_4KiB_PAGES_IN_1GiB) {
                continue;
            }

            // Guard: Ensure all NUMBER_OF_4KiB_PAGES_IN_1GiB bits starting here are completely free
            let out_of_bounds = word_idx >= self.bitmap.len();
            if out_of_bounds || !self.is_region_free(start_frame, NUMBER_OF_4KiB_PAGES_IN_1GiB) {
                continue;
            }

            // If we passed all guards, perform allocation and return
            self.mark_region_allocated(start_frame, NUMBER_OF_4KiB_PAGES_IN_1GiB);
            self.next_search_word = (start_frame + NUMBER_OF_4KiB_PAGES_IN_1GiB) / 64;

            let phys_addr = PhysAddr::new((start_frame * 4096) as u64);
            return Some(PhysFrame::containing_address(phys_addr));
        }

        None
    }
}

impl FrameDeallocator<Size1GiB> for BitMapFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size1GiB>) {
        let frame_index = (frame.start_address().as_u64() / Size4KiB::SIZE) as usize;
        self.mark_region_free(frame_index, NUMBER_OF_4KiB_PAGES_IN_1GiB);
        self.next_search_word = self.next_search_word.min(frame_index / 64);
    }
}
