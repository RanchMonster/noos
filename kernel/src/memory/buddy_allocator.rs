mod formulas;

use bootloader_api::info::{MemoryRegion, MemoryRegionKind, MemoryRegions};
use core::ptr::NonNull;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, PageSize, PhysFrame, Size1GiB, Size2MiB, Size4KiB,
    },
};

use crate::memory::bitmap_frame_allocator::BitMapFrameAllocator;

const PAGE_SIZE: usize = 4096; // 4 KiB Base Page
const HARD_MAX_ORDER: usize = 18; // Hard ceiling (1 GiB blocks)
/// Type alias for convenience
type NodePtr = NonNull<ListNode>;
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
    fn init(&mut self, memory_map: &'static [MemoryRegion], physical_offset: usize) {
        self.physical_offset = physical_offset;
        self.bitmap_allocator.init(memory_map, physical_offset);
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

        // 1) Mark the ENTIRE higher-order region as ALLOCATED in bitmap
        let frame = (node_addr(node) - self.physical_offset) / PAGE_SIZE;
        self.bitmap_allocator
            .mark_region_allocated(frame, formulas::order_to_size(next_order) / PAGE_SIZE);

        // 2) Split into sub-blocks, add ALL to lower order list
        for addr in (start_addr..end_addr).step_by(size_of_current_order) {
            let node = node_at!(addr);
            self.lists[order].prepend(node);
        }

        // 3) Mark the sub-blocks as FREE in bitmap (they're now in free list)
        for addr in (start_addr..end_addr).step_by(size_of_current_order) {
            let frame = (addr - self.physical_offset) / PAGE_SIZE;
            self.bitmap_allocator
                .mark_region_free(frame, size_of_current_order / PAGE_SIZE);
        }

        // 5) Return one block (pop from list we just filled)
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

    fn free_node(&mut self, phys_addr: PhysAddr, order: usize) {
        let number_of_pages = formulas::order_to_size(order) / PAGE_SIZE;
        let frame = (phys_addr.as_u64() / PAGE_SIZE as u64) as usize;

        self.bitmap_allocator
            .mark_region_free(frame, number_of_pages);

        let virt_addr = phys_addr.as_u64() as usize + self.physical_offset;
        let node = node_at!(virt_addr);
        unsafe {
            (*node.as_ptr()).next = core::ptr::null_mut();
            (*node.as_ptr()).prev = core::ptr::null_mut();
        }

        self.put_node_of_order(node, order);
    }
}

unsafe impl FrameAllocator<Size4KiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let order = formulas::size_to_order(Size4KiB::SIZE as usize);
        let node = self.get_node_of_order(order)?;

        let addr = node_addr(node);
        let frame = (addr - self.physical_offset) / PAGE_SIZE;
        let total_number_of_pages = formulas::order_to_size(order) / PAGE_SIZE;
        self.bitmap_allocator
            .mark_region_allocated(frame, total_number_of_pages);
        let phys_addr = self.convert_to_phys_addr(node);

        Some(PhysFrame::containing_address(phys_addr))
    }
}

unsafe impl FrameAllocator<Size2MiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size2MiB>> {
        let order = formulas::size_to_order(Size2MiB::SIZE as usize);
        let node = self.get_node_of_order(order)?;
        let addr = node_addr(node);
        let frame = (addr - self.physical_offset) / PAGE_SIZE;
        let total_number_of_pages = formulas::order_to_size(order) / PAGE_SIZE;
        self.bitmap_allocator
            .mark_region_allocated(frame, total_number_of_pages);
        let phys_addr = self.convert_to_phys_addr(node);

        Some(PhysFrame::containing_address(phys_addr))
    }
}

unsafe impl FrameAllocator<Size1GiB> for BuddyAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size1GiB>> {
        let order = formulas::size_to_order(Size1GiB::SIZE as usize);
        let node = self.get_node_of_order(order)?;
        let addr = node_addr(node);
        let frame = (addr - self.physical_offset) / PAGE_SIZE;
        let total_number_of_pages = formulas::order_to_size(order) / PAGE_SIZE;
        self.bitmap_allocator
            .mark_region_allocated(frame, total_number_of_pages);
        let phys_addr = self.convert_to_phys_addr(node);

        Some(PhysFrame::containing_address(phys_addr))
    }
}

impl FrameDeallocator<Size4KiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        let order = formulas::size_to_order(Size4KiB::SIZE as usize);
        self.free_node(frame.start_address(), order);
    }
}

impl FrameDeallocator<Size2MiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size2MiB>) {
        let order = formulas::size_to_order(Size2MiB::SIZE as usize);
        self.free_node(frame.start_address(), order);
    }
}

impl FrameDeallocator<Size1GiB> for BuddyAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size1GiB>) {
        let order = formulas::size_to_order(Size1GiB::SIZE as usize);
        self.free_node(frame.start_address(), order);
    }
}
