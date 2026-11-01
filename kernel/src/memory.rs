mod bitmap_frame_allocator;
mod buddy_allocator;
mod fixed_size_block;
mod formulas;
use core::{ops::Deref, slice};

use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use spin::Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageSize, PageTable,
        PageTableFlags, PhysFrame, Size1GiB, Size2MiB, Size4KiB,
    },
};

use crate::memory::{buddy_allocator::BuddyAllocator, fixed_size_block::FixedSizeBlockAllocator};
// The default page size the system will use to keep track of memory regions.
const PAGE_SIZE: usize = 4096;
pub static BUDDY_ALLOCATOR: Locked<BuddyAllocator> = Locked::new(BuddyAllocator::new());
#[global_allocator]
static ALLOCATOR: Locked<FixedSizeBlockAllocator> = Locked::new(FixedSizeBlockAllocator::new());

/// Initialize a new OffsetPageTable.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr, memory_map: &'static MemoryRegions) {
    let mut buddy = BUDDY_ALLOCATOR.lock();
    buddy.init(memory_map, physical_memory_offset.as_u64() as usize);
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

/// A wrapper around a spinlock for trait implementations.
/// This will likely be replaced/moved later, I plan to create my own mutex to optimize for
/// performance.
pub struct Locked<T> {
    inner: Mutex<T>,
}
impl<T> Locked<T> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}
impl<T> Deref for Locked<T> {
    type Target = Mutex<T>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
unsafe impl<T> Sync for Locked<T> {}
