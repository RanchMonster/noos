use core::arch::{asm, global_asm, naked_asm, x86_64::__cpuid_count};

use alloc::{boxed::Box, vec::Vec};
use x86_64::instructions::hlt;

use crate::{
    println, serial_println,
    task::{TaskContext, executor::Executor},
};
use core::ptr::write_volatile;
const LAPIC_BASE: usize = 0xFEE00000;
global_asm!(include_str!("trampoline.s"));
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuData {
    pub core_id: u32,
    pub _pad: u32,
    pub kernel_ctx: *mut TaskContext,
    pub executor: *mut Executor,
    pub switched: bool,
}

#[unsafe(naked)]
pub unsafe extern "C" fn set_gs_base(base: usize) {
    naked_asm!(
        "
        mov rcx, 0xC0000101     // IA32_GS_BASE MSR
        mov rax, rdi            // base (low 32 bits in eax)
        shr rdx, 32             // base >> 32 (high 32 bits in edx)
        wrmsr
        ret
        "
    );
}

#[unsafe(naked)]
pub unsafe extern "C" fn get_core_id_from_cpuid() -> u32 {
    naked_asm!(
        "
        mov eax, 0x0B       # CPUID leaf 0x0B (Extended Topology)
        xor ecx, ecx        # level 0 (current core)
        cpuid                # x2APIC ID is now in EDX
        mov eax, edx        # return core ID in EAX
        ret
        ",
    );
}
#[unsafe(naked)]
unsafe extern "C" fn _get_gs_base() -> usize {
    naked_asm!(
        "
        mov rcx, 0xC0000101     // IA32_GS_BASE MSR
        rdmsr
        shl rdx, 32             // high 32 bits in edx
        or rax, rdx             // return 64-bit value in rax
        ret
        "
    );
}
/// Get the base of the global descriptor table
pub unsafe extern "C" fn get_cpu_data() -> &'static CpuData {
    unsafe {
        let cpu_data_ptr = _get_gs_base() as *const CpuData;
        &*cpu_data_ptr
    }
}
/// Get the base of the global descriptor table
pub extern "C" fn get_core_id() -> u32 {
    unsafe { get_cpu_data().core_id }
}
/// Write to the local APIC
unsafe fn lapic_write(offset: usize, value: u32) {
    let reg = (LAPIC_BASE + offset) as *mut u32;
    unsafe { write_volatile(reg, value) };
}
/// Send INIT IPI to an AP
unsafe fn send_init_ipi(apic_id: u8) {
    // Write target APIC ID to high dword
    lapic_write(0x310, (apic_id as u32) << 24);
    // Write INIT command to low dword
    lapic_write(0x300, 0x4500); // INIT, level=assert, edge trigger
}

/// Send SIPI to an AP
unsafe fn send_sipi(apic_id: u8, vector: u8) {
    lapic_write(0x310, (apic_id as u32) << 24); // target APIC ID
    lapic_write(0x300, 0x4600 | (vector as u32)); // SIPI, level=assert, vector
}

#[inline(always)]
/// Get the current stack pointer from (RSP)
/// This is unsafe because it is not guaranteed that the current stack pointer is valid and is not
/// inteneded to be used in a public API instead use the cpu_data.kernel_ctx.rbp
pub unsafe fn current_stack_ptr() -> usize {
    let rsp: usize;
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nomem, nostack, preserves_flags)
        );
    }
    rsp
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_main() {
    unsafe {
        lapic_write(0xF0, 0x1FF);
        lapic_write(0x80, 0x10000);
        asm!("sti");
    }
    #[cfg(test)]
    serial_println!("Other CPU booted");
    #[cfg(not(test))]
    println!("Other CPU booted");
    hlt();
}
/// init the cpu data
pub fn init_cpu_data(stack: *mut u8) {
    let cpuid = unsafe { get_core_id_from_cpuid() };
    let mut kernel_ctx = TaskContext::empty();
    kernel_ctx.rbp = stack as usize;
    let executor = Executor::new();
    let cpu_data = Box::into_raw(Box::new(CpuData {
        core_id: cpuid,
        _pad: 0,
        kernel_ctx: Box::into_raw(Box::new(kernel_ctx)),
        executor: Box::into_raw(Box::new(executor)),
        switched: false,
    }));
    unsafe { set_gs_base(cpu_data as usize) };
}
#[cfg(test)]
pub fn test_core_init() {
    // init one other core
    unsafe { 
        send_init_ipi(1);
        // Wait 10ms after INIT
        for _ in 0..10000 {
            core::hint::spin_loop();
        }
        // Send SIPI with vector 0x08 (trampoline at 0x8000)
        send_sipi(1, 0x08);
        // Wait 200us
        for _ in 0..2000 {
            core::hint::spin_loop();
        }
        // Send second SIPI (as per Intel spec)
        send_sipi(1, 0x08);
    }
}
