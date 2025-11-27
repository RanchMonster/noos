use x86_64::instructions::port::Port;
/// I/O port for the Programmable Interrupt Timer (PIT)
const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
/// Base frequency of the PIT crystal
const PIT_BASE_HZ: u32 = 1_193_182;
const TIMER_HZ: u32 = 100; // 10ms granularity
/// Initialize the PIT to fire interrupts at 'TIMER_HZ' frequency
pub fn init() {
    let divisor = (PIT_BASE_HZ / TIMER_HZ) as u8;
    unsafe {
        let mut cmd = Port::new(PIT_COMMAND);
        cmd.write(0x36 as u8); // select counter 0
        let mut channel0 = Port::new(PIT_CHANNEL0);
        channel0.write(divisor as u8);
        channel0.write((divisor >> 8) as u8);
    }
}
