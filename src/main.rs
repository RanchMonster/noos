#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(noos::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::vec::Vec;
use bootloader::{BootInfo, bootinfo::MemoryRegionType, entry_point};
use core::{arch::asm, panic::PanicInfo};
use noos::{memory::BitMapFrameAllocator, println};
use x86_64::structures::paging::{PageSize, Size1GiB, Size2MiB};
const VERSION: &str = env!("CARGO_PKG_VERSION");
entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use noos::allocator;
    use noos::memory::{self, BootInfoFrameAllocator};
    use x86_64::VirtAddr;
    println!("Starting NoOS {}", VERSION);
    noos::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut boot_info_frame_allocator =
        unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };
    let mut bitmap_frame_allocator =
        BitMapFrameAllocator::new(&mut mapper, boot_info_frame_allocator);
    // initialize the heap for the kernel
    // for now, just allocate 10MiB of heap memory (enough for a single-threaded kernel)
    allocator::init_heap(&mut mapper, &mut bitmap_frame_allocator, 1024 * 1024 * 10)
        .expect("heap initialization failed");
    #[cfg(test)]
    test_main();
    loop {}
}
fn get_stack() -> *mut u8 {
    let stack;
    unsafe { asm!("mov {}, rsp", out(reg) stack) };
    stack
}
/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    noos::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    noos::test_panic_handler(info)
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
