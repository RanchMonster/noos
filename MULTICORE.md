# Starting Other CPU Cores in NOOS

Your project already has the infrastructure for multi-core support! Here's how to start additional cores.

## Overview

The project has three key components for multi-core support:

1. **APIC (Advanced Programmable Interrupt Controller)** - Located in `src/task/cpu_funcs.rs`
2. **Trampoline code** - Located in `src/task/trampoline.s` (real mode → 32-bit → 64-bit long mode)
3. **AP (Application Processor) main entry** - Located in `src/task/cpu_funcs.rs`

## How to Start Other Cores

### Step 1: Initialize CPU Data on the BSP (Boot Processor)

In your `main.rs` (kernel_main), call this to set up the BSP:

```rust
use noos::task::cpu_funcs::init_cpu_data;

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // ... existing initialization ...
    
    // Allocate and initialize BSP CPU data
    let bsp_stack = allocate_kernel_stack(); // You need to implement this
    init_cpu_data(bsp_stack as *mut u8);
    
    // ... rest of kernel_main ...
}
```

### Step 2: Start Additional Cores

The project has a `test_core_init()` function in `src/task/cpu_funcs.rs` that shows how to start a core:

```rust
use noos::task::cpu_funcs::{init_cpu_data, send_init_ipi, send_sipi};

fn start_ap_cores() {
    for apic_id in 1..num_cores {
        // Allocate stack for this AP
        let ap_stack = allocate_kernel_stack();
        
        // Initialize CPU data (needs to be at a known address)
        init_cpu_data(ap_stack as *mut u8);
        
        unsafe {
            // Send INIT IPI (Interrupt)
            send_init_ipi(apic_id as u8);
            
            // Wait 10ms after INIT
            for _ in 0..10000 {
                core::hint::spin_loop();
            }
            
            // Send SIPI (Startup IPI) with vector 0x08
            // This points to the trampoline code at address 0x8000
            send_sipi(apic_id as u8, 0x08);
            
            // Wait 200us
            for _ in 0..2000 {
                core::hint::spin_loop();
            }
            
            // Send second SIPI (Intel specification requires two)
            send_sipi(apic_id as u8, 0x08);
        }
    }
}
```

### Step 3: Set the Trampoline Parameters

Before sending SIPI, set the trampoline parameters:

```rust
// Set the page table address (needs to be physical address)
unsafe {
    let pml4_ptr = 0x8000 + 0x0018; // Offset in trampoline data section
    *(pml4_ptr as *mut u64) = page_table_physical_address;
    
    // Set the ap_main function pointer
    let ap_main_ptr = 0x8000 + 0x0020;
    *(ap_main_ptr as *mut u64) = ap_main as u64;
}
```

### Step 4: Handle AP Startup

The trampoline will jump to `ap_main()` (already defined in `src/task/cpu_funcs.rs`):

```rust
pub unsafe extern "C" fn ap_main() {
    unsafe {
        lapic_write(0xF0, 0x1FF);      // Enable APIC
        lapic_write(0x80, 0x10000);    // Disable timer
        asm!("sti");                    // Enable interrupts
    }
    
    println!("CPU {} booted", get_core_id());
    
    // Initialize this core
    noos::init();
    
    // Get the executor for this core and run it
    let executor = unsafe { &*get_cpu_data().executor };
    executor.run();
}
```

## Key Functions Already Implemented

| Function | Location | Purpose |
|----------|----------|---------|
| `init_cpu_data()` | cpu_funcs.rs | Set up per-core data (core ID, context, executor) |
| `send_init_ipi()` | cpu_funcs.rs | Send INIT IPI to start AP reset |
| `send_sipi()` | cpu_funcs.rs | Send startup IPI with vector address |
| `get_core_id()` | cpu_funcs.rs | Get current core's APIC ID |
| `set_gs_base()` | cpu_funcs.rs | Set per-core data via GS register |
| `ap_main()` | cpu_funcs.rs | AP entry point after trampoline |

## Important Considerations

1. **Bootloader Setup**: Ensure your bootloader supports:
   - APIC (check CPUID bit 9)
   - Memory mappings for APIC (0xFEE00000)

2. **Stack Allocation**: Each core needs its own kernel stack. Allocate from your heap/memory manager.

3. **GDT/IDT**: Each core gets its own GS base register pointing to its `CpuData` structure.

4. **Trampoline Location**: The trampoline must be loaded at address `0x8000` (8KB boundary).

5. **Synchronization**: Use the existing `Executor` and task queues with proper locks for thread safety.

## Testing Multi-Core

The project already has a test:

```bash
# Build and run with multi-core test
just run-custom "-smp 4"
```

The test calls `test_core_init()` which starts core 1. To start all cores, modify:

```rust
#[test_case]
fn test_multicore() {
    unsafe {
        // Initialize BSP
        let bsp_stack = allocate_stack();
        init_cpu_data(bsp_stack as *mut u8);
        
        // Start all APs
        for apic_id in 1..4 {
            let ap_stack = allocate_stack();
            init_cpu_data(ap_stack as *mut u8);
            
            send_init_ipi(apic_id);
            spin_sleep_ms(10);
            send_sipi(apic_id, 0x08);
            spin_sleep_ms(1);
            send_sipi(apic_id, 0x08);
        }
    }
    
    // Wait for APs to boot
    spin_sleep_ms(100);
}
```

## Next Steps

1. **Implement stack allocation** in your memory manager
2. **Map the trampoline** at 0x8000 in physical memory
3. **Detect CPU count** using CPUID or ACPI
4. **Integrate core startup** into your main kernel initialization
5. **Test with multiple cores**: `just run-custom "-smp 8"`
