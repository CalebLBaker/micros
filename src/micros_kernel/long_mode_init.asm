global long_mode_start
global launch_memory_manager
global breakpoint_handler
global spurious_interrupt_handler
global error_interrupt_handler
global timer_interrupt_handler
global double_fault_handler
global page_fault_handler

USER_DATA_SEGMENT equ 0x23
USER_CODE_SEGMENT equ 0x2b

END_OF_INTERRUPT equ 0xFEE000B0

section .text
bits 64
extern main

long_mode_start:
    mov rsp, 0

    ; null out segment registers
    mov ax, 0
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    call main

spurious_interrupt_handler:
error_interrupt_handler:
timer_interrupt_handler:
    push rax
    mov rax, END_OF_INTERRUPT
    mov dword [rax], 0
    pop rax
breakpoint_handler:
    iretq

halt:
    hlt
double_fault_handler:
page_fault_handler:
    jmp halt

; Args:
; rdi: virtual address of root Amd64FrameAllocator structure
; rsi: virtual address of the multiboot2 information struct
; rdx: physical address of root page table for memory manager
; rcx: virtual address of memory manager main function
launch_memory_manager:
    mov cr3, rdx
    mov rsp, 0
    mov ax, USER_DATA_SEGMENT
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push USER_DATA_SEGMENT
    push 0
    pushf
    push USER_CODE_SEGMENT
    push rcx
    iretq

