use alloc::alloc::{GlobalAlloc, Layout};
use core::{mem, ptr::NonNull};

use crate::memory::{BUDDY_ALLOCATOR, Locked, PAGE_SIZE, formulas};

/// The block sizes to use.
/// The sizes must each be power of 2 because they are also used as
/// the block alignment (alignments must be always powers of 2).
const BLOCK_SIZES: &[usize] = &[8, 16, 32, 64, 128, 256, 512, 1024, 2048];

/// Returns an index into the `BLOCK_SIZES` array.
fn list_index(layout: &Layout) -> Option<usize> {
    let required_block_size = layout.size().max(layout.align());
    BLOCK_SIZES.iter().position(|&s| s >= required_block_size)
}

const fn get_size_to_allocate_from_buddy(list_index: usize) -> usize {
    let block_size = BLOCK_SIZES[list_index];
    let size_of_page_header = mem::size_of::<PageHeader>();
    let size_to_allocate = (block_size * 8) + size_of_page_header;
    formulas::order_to_size(formulas::size_to_order(size_to_allocate))
}
/// Returns the address of the page header given the address of the block.
/// # Safety
/// This function is unsafe because it assumes that the given address is a valid pointer to a block
/// of memory.
const unsafe fn get_page_header(addr: usize, list_index: usize) -> Option<NonNull<PageHeader>> {
    let at_addr = addr & !(get_size_to_allocate_from_buddy(list_index) - 1);
    NonNull::new(at_addr as *mut _)
}

struct BlockNode {
    next: Option<NonNull<BlockNode>>,
}

#[derive(Debug, Clone, Copy)]
struct PageHeader {
    total_blocks: usize,
    free_blocks: usize,
    next_block: Option<NonNull<BlockNode>>,
    next_page_header: Option<NonNull<PageHeader>>,
}

pub struct FixedSizeBlockAllocator {
    list_heads: [Option<NonNull<PageHeader>>; BLOCK_SIZES.len()],
}

impl FixedSizeBlockAllocator {
    /// Creates an empty FixedSizeBlockAllocator.
    pub const fn new() -> Self {
        const EMPTY: Option<NonNull<PageHeader>> = None;
        FixedSizeBlockAllocator {
            list_heads: [EMPTY; BLOCK_SIZES.len()],
        }
    }
    fn get_block(&mut self, list_index: usize) -> Option<NonNull<BlockNode>> {
        if let Some(mut head) = self.list_heads[list_index].as_mut() {
            while unsafe { head.as_mut().free_blocks == 0 && head.as_mut().next_block.is_some() } {
                head = unsafe {
                    head.as_mut()
                        .next_page_header
                        .as_mut()
                        .expect("This should never happen")
                };
            }
            let head_ref = unsafe { head.as_mut() };

            if head_ref.free_blocks > 0 {
                head_ref.free_blocks -= 1;
                return Some(
                    head_ref
                        .next_block
                        .take()
                        .expect("Page header reported free blocks"),
                );
            }
        }
        let must_allocate = get_size_to_allocate_from_buddy(list_index);
        let layout = Layout::from_size_align(must_allocate, PAGE_SIZE)
            .expect("Should be able to align to page");
        let ptr = BUDDY_ALLOCATOR.lock().alloc(layout)?;

        let mut page_header_ptr: NonNull<PageHeader> = ptr.cast();

        // This closuer is used to assert that my get_page_header function works as expected.
        {
            let some_addrs_in_block = unsafe { ptr.add(1024) };
            let page_header_ptr_from_math =
                unsafe { get_page_header(some_addrs_in_block.addr().get(), list_index) };

            assert!(
                page_header_ptr_from_math.is_some_and(|item| item.addr() == page_header_ptr.addr()),
                "These should be the same"
            )
        }

        let page_header_ref = unsafe { page_header_ptr.as_mut() };

        let block_size = BLOCK_SIZES[list_index];

        let alignment_offset = ptr.addr().get() & (block_size - 1);

        let aligned_ptr = unsafe { ptr.cast::<u8>().add(alignment_offset) };

        let end_of_page = unsafe { ptr.add(must_allocate) };

        // TLDR: Create blocks of the block_size from the usable space
        for block in (aligned_ptr.addr()..end_of_page.addr()).step_by(block_size) {
            let mut block_ptr: NonNull<BlockNode> =
                unsafe { NonNull::new_unchecked(block.get() as *mut _) };

            let block_ref = unsafe { block_ptr.as_mut() };

            let next = page_header_ref.next_block.take();
            block_ref.next = next;
            page_header_ref.next_block = Some(block_ptr);

            page_header_ref.total_blocks += 1;
        }

        page_header_ref.free_blocks = page_header_ref.total_blocks - 1;
        let usable_block = page_header_ref
            .next_block
            .expect("There should be at least one usable block");

        // TLDR: Add the new page header to the list heads
        let current_head = self.list_heads[list_index];
        page_header_ref.next_page_header = current_head;
        self.list_heads[list_index] = Some(page_header_ptr);

        Some(usable_block)
    }

    /// Put a block back in the Page it belongs to.
    /// # Safety
    /// This function assumes that the block pointer points to a valid block node.
    unsafe fn put_block(&mut self, list_index: usize, mut block: NonNull<BlockNode>) {
        assert!(
            list_index <= BLOCK_SIZES.len(),
            "The list index should not be out of bounds"
        );
        let block_ref = unsafe { block.as_mut() };

        let page_header_ref = unsafe {
            get_page_header(block.addr().get(), list_index)
                .expect("Page header should be a valid addrs")
                .as_mut()
        };

        let next = page_header_ref.next_block.take();
        block_ref.next = next;
        page_header_ref.next_block = Some(block);
        page_header_ref.free_blocks += 1;
        assert!(
            page_header_ref.total_blocks <= page_header_ref.free_blocks,
            "The Page header will never have more free blocks then total blocks"
        )
    }
}

unsafe impl GlobalAlloc for Locked<FixedSizeBlockAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(list_index) = list_index(&layout) else {
            return BUDDY_ALLOCATOR
                .lock()
                .alloc(layout)
                .expect("Out of Memory")
                .as_ptr();
        };
        let mut this = self.lock();
        this.get_block(list_index)
            .expect("Out of Memory")
            .cast()
            .as_ptr()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let Some(ptr) = NonNull::new(ptr) else {
            return;
        };

        let Some(list_index) = list_index(&layout) else {
            unsafe {
                return BUDDY_ALLOCATOR.lock().dealloc(ptr, layout);
            }
        };
        let mut this = self.lock();
        unsafe { this.put_block(list_index, ptr.cast()) }
    }
}
