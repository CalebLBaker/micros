global long_mode_start
global launch_memory_manager

USER_DATA_SEGMENT equ 0x23
USER_CODE_SEGMENT equ 0x2b

section .text
bits 64
extern main
extern halt
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

; Args:
; rdi: virtual address of root Amd64FrameAllocator structure
; rsi: physical address of root page table for memory manager
; rdx: virtual address of memory manager main function
launch_memory_manager:
	mov cr3, rsi
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
	push rdx
	iretq

