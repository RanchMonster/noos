use core::sync::atomic::AtomicU64;

use x86_64::instructions::port::Port;
/// I/O port for the Programmable Interrupt Timer (PIT)
const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
/// Base frequency of the PIT crystal
const PIT_BASE_HZ: u32 = 1_193_182;
const TIMER_HZ: u32 = 1000; // Timer frequency
pub static TICKS: AtomicU64 = AtomicU64::new(0);

/// Initialize the PIT to fire interrupts at 'TIMER_HZ' frequency
pub fn init() {
    let divisor: u16 = (PIT_BASE_HZ / TIMER_HZ) as u16;
    unsafe {
        let mut cmd = Port::new(PIT_COMMAND);
        cmd.write(0x36_u8); // channel 0, lobyte/hibyte, mode 3, binary

        let mut channel0 = Port::new(PIT_CHANNEL0);
        channel0.write((divisor & 0xFF) as u8); // low byte
        channel0.write((divisor >> 8) as u8); // high byte
    }
}

/// Ticks the current counter by 1
pub fn do_tick() {
    TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}
