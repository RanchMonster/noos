#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(noos::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::vec::Vec;
use bootloader::{BootInfo, bootinfo::MemoryRegionType, entry_point};
use core::panic::PanicInfo;
use noos::println;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use noos::allocator;
    use noos::memory::{self, BootInfoFrameAllocator};
    use x86_64::VirtAddr;

    println!("Hello World{}", "!");
    noos::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };
    let mut heap_size: usize = 0;
    for frame in boot_info.memory_map.iter() {
        if frame.region_type == MemoryRegionType::Usable {
            heap_size += (frame.range.end_addr() - frame.range.start_addr()) as usize;
        }
    }
    println!("heap size: {}", heap_size);
    allocator::init_heap(&mut mapper, &mut frame_allocator, heap_size)
        .expect("heap initialization failed");
    #[cfg(test)]
    test_main();
    loop {}
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
