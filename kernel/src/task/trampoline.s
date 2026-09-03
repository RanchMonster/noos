# AP trampoline code - Intel syntax for Rust global_asm
.intel_syntax noprefix
.code16
.section .text
.global ap_trampoline
ap_trampoline:
   cli
   xor ax, ax
   mov ds, ax
   lgdt [gdt_ptr_real]
   mov eax, cr0
   or eax, 1
   mov cr0, eax
   # Far jump to 32-bit protected mode - manual encoding
   .byte 0xEA              # Far jump opcode
   .long pm_entry          # Offset
   .word 0x08              # Segment selector

.code32
pm_entry:
   mov ax, 0x10
   mov ds, ax
   mov es, ax
   mov fs, ax
   mov gs, ax
   mov ss, ax

   # Enable PAE
   mov eax, cr4
   or eax, 0x20
   mov cr4, eax

   # Load page table (needs to be set before starting APs)
   mov eax, [pml4_phys]
   mov cr3, eax
   
   # Enable Long Mode in EFER
   mov ecx, 0xC0000080
   rdmsr
   or eax, 0x100
   wrmsr
   
   # Enable paging
   mov eax, cr0
   or eax, 0x80000000
   mov cr0, eax
   
   # Far jump to 64-bit long mode - manual encoding
   .byte 0xEA              # Far jump opcode  
   .long ap_entry_64       # Offset
   .word 0x08              # Segment selector

.code64
ap_entry_64:
   # Jump to ap_main function
   lea rax, [rip + ap_main_ptr]
   mov rax, [rax]
   jmp rax

# Data section
.align 8
gdt_ptr_real:
   .word gdt_end - gdt_start - 1
   .long gdt_start

.align 16
gdt_start:
   .quad 0                           # Null descriptor
   .quad 0x00CF9A000000FFFF         # Code segment (32/64-bit)
   .quad 0x00CF92000000FFFF         # Data segment
gdt_end:

pml4_phys:
   .quad 0

ap_main_ptr:
   .quad 0




