global long_mode_start
global launch_memory_manager
global breakpoint_handler
global spurious_interrupt_handler
global error_interrupt_handler
global timer_interrupt_handler
global double_fault_handler
global page_fault_handler
global write_port;
global set_apic_base;
global enable_interrupts
global load_tss
global reset_code_segment
global load_gdt
global load_idt
global halt

USER_DATA_SEGMENT equ 0x23
USER_CODE_SEGMENT equ 0x2B

END_OF_INTERRUPT equ 0xFEE000B0

APIC_BASE_MODEL_SPECIFIC_REGISTER equ 0x1B
APIC_BASE equ 0xFEE00800

CODE_SEGMENT equ 8
TSS equ 0x10

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
; di: port to write to
; sil: value to write
write_port:
    mov dx, di
    mov al, sil
    out dx, al
    ret

set_apic_base:
    mov ecx, APIC_BASE_MODEL_SPECIFIC_REGISTER
    mov eax, APIC_BASE
    mov edx, 0
    wrmsr
    ret

enable_interrupts:
    sti
    ret

; Args:
; rdi: physical address of idt pointer
load_idt:
   lidt [rdi]
   ret

; Args:
; rdi: physical address of gdt pointer
load_gdt:
   lgdt [rdi]
   ret

load_tss:
    mov ax, TSS
    ltr ax
return:
    ret

reset_code_segment:
    push qword CODE_SEGMENT
    push qword return
    retfq

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

