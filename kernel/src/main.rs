#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader_api::{
    BootInfo, BootloaderConfig,
    entry_point,
};
use core::panic::PanicInfo;
use kernel::{memory::BitMapFrameAllocator, println};
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(bootloader_api::config::Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    use kernel::allocator;
    use kernel::memory::{self, BootInfoFrameAllocator};
    use x86_64::VirtAddr;
    let physical_memory_offset = boot_info
        .physical_memory_offset
        .clone()
        .take()
        .expect("Physical memory offset not found");
    let mut offset_page_tabe =
        unsafe { memory::init(VirtAddr::new(physical_memory_offset)) };
    let boot_info_frame_allocator =
        unsafe { BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    let mut bitmap_frame_allocator =
        BitMapFrameAllocator::new(&mut offset_page_tabe, boot_info_frame_allocator);
    allocator::init_heap(&mut offset_page_tabe, &mut bitmap_frame_allocator)
        .expect("failed to initialize heap");
    let _vga_buffer = boot_info.framebuffer.take().expect("no framebuffer");

    #[cfg(test)]
    test_main();
    loop {}
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    #[cfg(debug_assertions)]
    println!("PANIC: {:?}", info);
    println!("{}", info);
    kernel::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::test_panic_handler(info)
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
