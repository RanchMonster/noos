use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use core::{alloc::Layout, ptr::NonNull};
use x86_64::{
    PhysAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, PageSize, PhysFrame, Size1GiB, Size2MiB, Size4KiB,
    },
};

use super::{bitmap_frame_allocator::BitMapFrameAllocator, formulas};

const PAGE_SIZE: usize = 4096; // 4 KiB Base Page
const HARD_MAX_ORDER: usize = 18; // Hard ceiling (1 GiB blocks)
/// Type alias for convenience
#[derive(Debug, PartialEq, Eq)]
struct ListNode {
    next: *mut ListNode,
    prev: *mut ListNode,
}
// Assert that PAGE_SIZE is a multiple of alignment of the ListNode
const _: () = assert!(
    PAGE_SIZE % align_of::<ListNode>() == 0,
    "PAGE_SIZE must be a multiple of align_of<ListNode>"
);

#[inline]
fn node_addr(node: NonNull<ListNode>) -> usize {
    node.as_ptr() as usize
}

macro_rules! node_at {
    ($addr:expr) => {{
        let addr = $addr;
        assert!(
            addr % align_of::<ListNode>() == 0,
            "The address given isn't ListNode aligned"
        );
        unsafe { NonNull::new_unchecked(addr as *mut ListNode) }
    }};
}

struct DoubleLinkedList {
    head: *mut ListNode,
    length: usize,
}
impl DoubleLinkedList {
    pub const fn new() -> Self {
        DoubleLinkedList {
            head: core::ptr::null_mut(),
            length: 0,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.head.is_null()
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn prepend(&mut self, node: NonNull<ListNode>) {
        let node_ptr = node.as_ptr();
        unsafe {
            (*node_ptr).prev = core::ptr::null_mut();
            (*node_ptr).next = self.head;
            if !self.head.is_null() {
                (*self.head).prev = node_ptr;
            }
            self.head = node.as_ptr();
        }
    }
    pub fn pop(&mut self) -> Option<NonNull<ListNode>> {
        if self.head.is_null() {
            return None;
        }
        let head = self.head;
        unsafe {
            self.head = (*head).next;
            if !self.head.is_null() {
                (*self.head).prev = core::ptr::null_mut();
            }
            (*head).next = core::ptr::null_mut();
            (*head).prev = core::ptr::null_mut();
            NonNull::new(head)
        }
    }

    unsafe fn remove(&mut self, node: NonNull<ListNode>) {
        let node_ptr = node.as_ptr();
        let prev;
        let next;
        unsafe {
            prev = (*node_ptr).prev;
            next = (*node_ptr).next;
        }

        if !prev.is_null() {
            unsafe {
                (*prev).next = next;
            }
        } else {
            self.head = next;
        }
        if !next.is_null() {
            unsafe {
                (*next).prev = prev;
            }
        }
        unsafe {
            (*node_ptr).next = core::ptr::null_mut();
            (*node_ptr).prev = core::ptr::null_mut();
        }
        self.length -= 1;
    }
}
/// The buddy allocator is the main allocator used by the kernel.
/// for more information about what a buddy allocator is, check out
/// https://en.wikipedia.org/wiki/Buddy_memory_allocation
pub struct BuddyAllocator {
    lists: [DoubleLinkedList; HARD_MAX_ORDER + 1],
    physical_offset: usize,
    bitmap_allocator: BitMapFrameAllocator,
}

impl BuddyAllocator {
    pub const fn new() -> Self {
        const EMPTY: DoubleLinkedList = DoubleLinkedList {
            head: core::ptr::null_mut(),
            length: 0,
        };
        BuddyAllocator {
            lists: [EMPTY; HARD_MAX_ORDER + 1],
            physical_offset: 0,
            bitmap_allocator: BitMapFrameAllocator::new(),
        }
    }
    fn map_region_to_order(&mut self, region: MemoryRegion) {
        let phys_start = region.start as usize;
        let phys_end = region.end as usize;
        assert!(
            phys_start % PAGE_SIZE == 0 && phys_end % PAGE_SIZE == 0,
            "Memory regions are not page aligned"
        );

        let mut addr = phys_start;
        while addr < phys_end {
            let remaining = phys_end - addr;

            // The block size is PAGE_SIZE << order, so the address needs at least
            // `order + 12` trailing zero bits to be aligned for that order.
            let align_order = (addr.trailing_zeros() as usize).saturating_sub(12);
            let mut order = align_order.min(HARD_MAX_ORDER);
            while order > 0 && formulas::order_to_size(order) > remaining {
                order -= 1;
            }

            let block_size = formulas::order_to_size(order);
            if self
                .bitmap_allocator
                .is_region_free(addr / PAGE_SIZE, block_size / PAGE_SIZE)
            {
                let node = node_at!(addr + self.physical_offset);
                unsafe {
                    (*node.as_ptr()).next = core::ptr::null_mut();
                    (*node.as_ptr()).prev = core::ptr::null_mut();
                }
                self.lists[order].prepend(node);
            }

            addr += block_size;
        }
    }
    pub fn init(&mut self, memory_map: &'static [MemoryRegion], physical_offset: usize) {
        self.physical_offset = physical_offset;
        self.bitmap_allocator.init(memory_map, physical_offset);
        
        for region in memory_map.iter().filter(|r| r.kind == MemoryRegionKind::Usable) {
            self.map_region_to_order(*region);
        }
    }
    fn get_node_of_order(&mut self, order: usize) -> Option<NonNull<ListNode>> {
        if let Some(node) = self.lists[order].pop() {
            return Some(node);
        }

        if order >= HARD_MAX_ORDER {
            return None;
        }

        let next_order = order + 1;

        let size_of_current_order = formulas::order_to_size(order);

        // This allows me to easily go through the order of nodes to find one that is free and can
        // be split
        let node = self.get_node_of_order(next_order)?;
        let start_addr = node_addr(node);
        let end_addr = start_addr + formulas::order_to_size(next_order);

        // Split into sub-blocks, add ALL to lower order list
        for addr in (start_addr..end_addr).step_by(size_of_current_order) {
            let node = node_at!(addr);
            self.lists[order].prepend(node);
        }

        self.lists[order].pop()
    }

    fn put_node_of_order(&mut self, mut node: NonNull<ListNode>, mut order: usize) {
        unsafe {
            (*node.as_ptr()).next = core::ptr::null_mut();
            (*node.as_ptr()).prev = core::ptr::null_mut();
        }

        while order < HARD_MAX_ORDER {
            let addr = node_addr(node);
            let block_size = formulas::order_to_size(order);

            assert!(
                addr % block_size == 0,
                "Node must be aligned to its block size"
            );

            let frame = (addr - self.physical_offset) / PAGE_SIZE;
            let buddy_frame = frame ^ (block_size / PAGE_SIZE);

            if !self
                .bitmap_allocator
                .is_region_free(buddy_frame, block_size / PAGE_SIZE)
            {
                break;
            }

            let buddy_addr = buddy_frame * PAGE_SIZE + self.physical_offset;
            let buddy = unsafe { NonNull::new_unchecked(buddy_addr as *mut ListNode) };

            unsafe { self.lists[order].remove(buddy) };

            node = unsafe { NonNull::new_unchecked(addr.min(buddy_addr) as *mut ListNode) };
            unsafe {
                (*node.as_ptr()).next = core::ptr::null_mut();
                (*node.as_ptr()).prev = core::ptr::null_mut();
            }

            order += 1;
        }

        self.lists[order].prepend(node);
    }

    fn convert_to_phys_addr(&self, node: NonNull<ListNode>) -> PhysAddr {
        let virt_addr = node_addr(node);
        let phys_addr = PhysAddr::new((virt_addr - self.physical_offset) as u64);
        phys_addr
    }

    /// Allocate a block of memory.
    /// This function will allocate a block of memory of the given size or closest order.
    pub fn alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        if layout.size() == 0 {
            return Some(NonNull::<u8>::dangling());
        }

        let required_size = layout.size().max(layout.align());
        let order = formulas::size_to_order(required_size);
        let block_size = formulas::order_to_size(order);

        let node = self.get_node_of_order(order)?;
        let byte_ptr = node.as_ptr() as *mut u8;

        unsafe { core::ptr::write_bytes(byte_ptr, 0, block_size); }

        let addr = node_addr(node);
        let frame = (addr - self.physical_offset) / PAGE_SIZE;
        let total_pages = block_size / PAGE_SIZE;
        self.bitmap_allocator.mark_region_allocated(frame, total_pages);

        unsafe { Some(NonNull::new_unchecked(byte_ptr)) }
    }
    /// Deallocates a block of memory previously allocated by this allocator.
    /// # Safety
    /// This function is unsafe because it can cause undefined behavior if the given pointer is not
    /// a valid pointer to a block of memory previously allocated by this allocator.
    pub unsafe fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let required_size = layout.size().max(layout.align());
        let order = formulas::size_to_order(required_size);
        let block_size = formulas::order_to_size(order);

        let block_start = (ptr.as_ptr() as usize) & !(block_size - 1);
        assert!(block_start % block_size == 0, "Pointer not aligned to block size");
        let frame = (block_start - self.physical_offset) / PAGE_SIZE;
        assert!(self.bitmap_allocator.is_region_used(frame, block_size / PAGE_SIZE), 
                "Double-free or invalid pointer");
        let node = node_at!(block_start);
        self.put_node_of_order(node, order);
        self.bitmap_allocator.mark_region_free(frame, block_size / PAGE_SIZE);
    }
}

unsafe impl FrameAllocator<Size4KiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.alloc(unsafe {
            Layout::from_size_align_unchecked(Size4KiB::SIZE as usize, Size4KiB::SIZE as usize)
        })
        .map(|ptr| {
            let phys_addr = ptr.as_ptr() as u64 - self.physical_offset as u64;
            PhysFrame::containing_address(PhysAddr::new(phys_addr))
        })
    }
}

unsafe impl FrameAllocator<Size2MiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size2MiB>> {
        self.alloc(unsafe {
            Layout::from_size_align_unchecked(Size2MiB::SIZE as usize, Size2MiB::SIZE as usize)
        })
        .map(|ptr| {
            let phys_addr = ptr.as_ptr() as u64 - self.physical_offset as u64;
            PhysFrame::containing_address(PhysAddr::new(phys_addr))
        })
    }
}

unsafe impl FrameAllocator<Size1GiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size1GiB>> {
        self.alloc(unsafe {
            Layout::from_size_align_unchecked(Size1GiB::SIZE as usize, Size1GiB::SIZE as usize)
        })
        .map(|ptr| {
            let phys_addr = ptr.as_ptr() as u64 - self.physical_offset as u64;
            PhysFrame::containing_address(PhysAddr::new(phys_addr))
        })
    }
}

impl FrameDeallocator<Size4KiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        let phys_addr = frame.start_address();
        let virt_addr = phys_addr.as_u64() + self.physical_offset as u64;

        unsafe {
            self.dealloc(
                NonNull::new_unchecked(virt_addr as *mut u8),
                Layout::from_size_align_unchecked(Size4KiB::SIZE as usize, Size4KiB::SIZE as usize),
            )
        };
    }
}

impl FrameDeallocator<Size2MiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size2MiB>) {
        let phys_addr = frame.start_address();
        let virt_addr = phys_addr.as_u64() + self.physical_offset as u64;

        unsafe {
            self.dealloc(
                NonNull::new_unchecked(virt_addr as *mut u8),
                Layout::from_size_align_unchecked(Size2MiB::SIZE as usize, Size2MiB::SIZE as usize),
            )
        };
    }
}

impl FrameDeallocator<Size1GiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size1GiB>) {
        let phys_addr = frame.start_address();
        let virt_addr = phys_addr.as_u64() + self.physical_offset as u64;

        unsafe {
            self.dealloc(
                NonNull::new_unchecked(virt_addr as *mut u8),
                Layout::from_size_align_unchecked(Size1GiB::SIZE as usize, Size1GiB::SIZE as usize),
            )
        };
    }
}
