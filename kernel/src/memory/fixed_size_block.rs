use alloc::alloc::{GlobalAlloc, Layout};
use core::{
    mem,
    ptr::NonNull,
};

use crate::memory::{BUDDY_ALLOCATOR, Locked, PAGE_SIZE};

/// The block sizes to use.
/// The sizes must each be power of 2 because they are also used as
/// the block alignment (alignments must be always powers of 2).
const BLOCK_SIZES: &[usize] = &[8, 16, 32, 64, 128, 256, 512, 1024, 2048];

///
/// Returns an index into the `BLOCK_SIZES` array.
fn list_index(layout: &Layout) -> Option<usize> {
    let required_block_size = layout.size().max(layout.align());
    BLOCK_SIZES.iter().position(|&s| s >= required_block_size)
}

struct ListNode {
    next: Option<NonNull<ListNode>>,
}
/// Wrapper around the first 16 bits of a Page given to this allocator from the BuddyAllocator.
#[repr(transparent)]
struct PageMetaData(u16);

const _: () = assert!(
    mem::size_of::<PageMetaData>() == mem::size_of::<u16>(),
    "PageMetaData must be 16 bits"
);

pub struct FixedSizeBlockAllocator {
    list_heads: [Option<NonNull<ListNode>>; BLOCK_SIZES.len()],
}

impl FixedSizeBlockAllocator {
    /// Creates an empty FixedSizeBlockAllocator.
    pub const fn new() -> Self {
        const EMPTY: Option<NonNull<ListNode>> = None;
        FixedSizeBlockAllocator {
            list_heads: [EMPTY; BLOCK_SIZES.len()],
        }
    }

    fn fallblock_alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let required = layout.size().max(layout.align());
        let idx = BLOCK_SIZES.iter().position(|&s| s >= required)?;
        let block_size = BLOCK_SIZES[idx];

        if let Some(node) = self.list_heads[idx].take() {
            self.list_heads[idx] = unsafe { node.as_ref().next };
            let page_start = (node.as_ptr() as usize) & !(PAGE_SIZE - 1);
            let meta = page_start as *mut PageMetaData;
            unsafe { 
                assert!((*meta).0 > 0, "PageMetaData underflow: double-free or invalid pointer");
                (*meta).0 -= 1; 
            }
            return Some(node.cast::<u8>());
        }

        let page_layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };
        let page_ptr = BUDDY_ALLOCATOR.lock().alloc(page_layout)?;
        let page_start = page_ptr.as_ptr() as usize;

        let blocks_per_page = (PAGE_SIZE - 2) / block_size;
        let meta = page_start as *mut PageMetaData;
        unsafe { (*meta).0 = blocks_per_page as u16; }

        let first_block = page_start + 2;
        let aligned_first = first_block.next_multiple_of(block_size);

        let mut head: Option<NonNull<ListNode>> = None;
        let mut prev: Option<NonNull<ListNode>> = None;
        for i in 0..blocks_per_page {
            let block_addr = aligned_first + i * block_size;
            let mut node = unsafe { NonNull::new_unchecked(block_addr as *mut ListNode) };
            unsafe { node.as_mut().next = None; }
            if let Some(mut p) = prev {
                unsafe { p.as_mut().next = Some(node); }
            } else {
                head = Some(node);
            }
            prev = Some(node);
        }

        let mut allocated = head.unwrap();
        head = unsafe { allocated.as_mut().next.take() };
        self.list_heads[idx] = head;

        unsafe { (*meta).0 -= 1; }

        Some(allocated.cast::<u8>())
    }

    fn fallblock_dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let required = layout.size().max(layout.align());
        let idx = match BLOCK_SIZES.iter().position(|&s| s >= required) {
            Some(i) => i,
            None => {
                let page_layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };
                unsafe { 
                    BUDDY_ALLOCATOR.lock().dealloc(ptr, page_layout); 
                }
                return;
            }
        };

        let block_size = BLOCK_SIZES[idx];
        let page_start = (ptr.as_ptr() as usize) & !(PAGE_SIZE - 1);
        let meta = page_start as *mut PageMetaData;

        let blocks_per_page = (PAGE_SIZE - 2) / block_size;
        unsafe { 
            assert!((*meta).0 < blocks_per_page as u16, "PageMetaData overflow: double-free?");
            (*meta).0 += 1; 
        }

        if unsafe { (*meta).0 as usize == blocks_per_page } {
            let mut new_head = None;
            let mut current = self.list_heads[idx].take();
            while let Some(mut node) = current {
                let node_page = (node.as_ptr() as usize) & !(PAGE_SIZE - 1);
                current = unsafe { node.as_ref().next };
                if node_page != page_start {
                    unsafe { node.as_mut().next = new_head; }
                    new_head = Some(node);
                }
            }
            self.list_heads[idx] = new_head;

            let page_layout = unsafe { Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE) };
            unsafe { 
                BUDDY_ALLOCATOR.lock().dealloc(NonNull::new_unchecked(page_start as *mut u8), page_layout); 
            }
        } else {
            let mut node = unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut ListNode) };
            unsafe { node.as_mut().next = self.list_heads[idx].take(); }
            self.list_heads[idx] = Some(node);
        }
    }
}

unsafe impl GlobalAlloc for Locked<FixedSizeBlockAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut allocator = self.lock();
        allocator.fallblock_alloc(layout)
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut allocator = self.lock();
        if let Some(p) = NonNull::new(ptr) {
            allocator.fallblock_dealloc(p, layout);
        }
    }
}
