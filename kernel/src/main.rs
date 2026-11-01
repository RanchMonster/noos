#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
use alloc::vec::Vec;
use bootloader_api::{BootInfo, BootloaderConfig, entry_point};
use core::panic::PanicInfo;
use kernel::{memory, println};
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(bootloader_api::config::Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    use x86_64::VirtAddr;
    let physical_memory_offset = boot_info
        .physical_memory_offset
        .clone()
        .take()
        .expect("Physical memory offset not found");
    // Init the kernel before we init memory or start running userspace programs
    kernel::init();
    // Init all system memory that can be used for heap
    unsafe {
        memory::init(
            VirtAddr::new(physical_memory_offset),
            &boot_info.memory_regions,
        );
    }

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
