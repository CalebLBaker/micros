; Declare constants for the multiboot header
MULTIBOOT_2   equ 0xe85250d6 ; 'magic number' lets bootloader find the header
X86           equ 0          ; architecture enum value for x86
HEADER_LENGTH equ header_end - header_start

; Miscelaneous constants
MULTIBOOT_CHECK       equ 0x36d76289
CPUID_BIT             equ 0x200000
GIGABYTE_PAGES_CPUID_BIT equ 0x4000000
LONG_MODE_CPUID_BIT   equ 0x20000000
PAGE_SIZE             equ 0x1000
PAGE_TABLE_ENTRY_SIZE equ 8
GIGABYTE              equ 0x40000000
NUM_P2_TABLES         equ 4
PAGE_TABLE_FLAGS      equ 7
PAGE_FLAGS equ (0x80 + PAGE_TABLE_FLAGS)
LAST_PAGE_TABLE_ENTRY equ PAGE_SIZE - PAGE_TABLE_ENTRY_SIZE
PHYSICAL_ADDRESS_EXPANSION equ 0x20
EFER_LONG_MODE_NO_EXECUTE equ 0x900
EFER_MSR              equ 0xC0000080
PAGING_FLAG           equ 0x80000000
LONG_CODE_SEGMENT     equ 0x20980000000000
MODULE_ALIGNMENT_TAG  equ 6
MULTIBOOT_END_TAG     equ 0

; Declare a multiboot header that marks the program as a kernel.
; Format is documented in the multiboot standard
; Must be in the first 8 kB of kernel file, 32-bit aligned
global header_start
section .multiboot_header
align 8
header_start:
    dd MULTIBOOT_2
    dd X86
    ; header length
    dd HEADER_LENGTH
    ; checksum
    dd -(MULTIBOOT_2 + X86 + HEADER_LENGTH)

	; module alignment tag
	dw MODULE_ALIGNMENT_TAG ; type
	dw 0 ; flags
	dd 8 ; size

    ; required end tag
    dw MULTIBOOT_END_TAG ; type
    dw 0 ; flags
    dd 8 ; size
header_end:

global p4_table
global p2_tables
global p1_table_for_stack
section .bss
; Stack
align PAGE_SIZE
stack_bottom:
    resb 0x2000

; Page tables
p4_table:
    resb PAGE_SIZE
p3_table:
    resb PAGE_SIZE
; Allow 4 p2 tables so we can identity map 4 GB
p2_tables:
    resb NUM_P2_TABLES * PAGE_SIZE
p3_table_for_stack:
    resb PAGE_SIZE
p2_table_for_stack:
    resb PAGE_SIZE
p1_table_for_stack:
	resb PAGE_SIZE

section .rodata
gdt64:
    dq 0
.code: equ $ - gdt64
    dq LONG_CODE_SEGMENT
.pointer:
    dw $ - gdt64 - 1
    dq gdt64

; Entry point
global _start
extern long_mode_start
section .text
bits 32
_start:

    ; Make sure the kernel was loaded by a multiboot compliant bootloader
    cmp eax, MULTIBOOT_CHECK
    jne stop

    ; Make sure cpuid is supported
    pushfd
    pop eax
    mov ecx, eax
    xor eax, CPUID_BIT
    push eax
    popfd
    pushfd
    pop eax
    push ecx
    popfd
    cmp eax, ecx
    je stop

    ; Stash ebx to edi before calling cpuid
    mov edi, ebx

    ; Make sure long mode (64 bit mode) is supported
    mov eax, 0x80000000
    cpuid
    cmp eax, 0x80000001
    jb stop
    mov eax, 0x80000001
    cpuid
    test edx, LONG_MODE_CPUID_BIT
    jz stop

    ; Save cpuid result into esi so it can be passed into main
    mov esi, edx

    ; Set up p4 table with 1 entry
    mov eax, p3_table
    or eax, PAGE_TABLE_FLAGS
    mov [p4_table], eax

	; Test for GB page support
	test esi, GIGABYTE_PAGES_CPUID_BIT
	jz .map_p3_table_no_gigabyte_pages

    ; Set up p3 table with 4 entries
    mov eax, PAGE_FLAGS
    mov [p3_table], eax
    add eax, GIGABYTE
    mov [p3_table + PAGE_TABLE_ENTRY_SIZE], eax
    add eax, GIGABYTE
    mov [p3_table + 2 * PAGE_TABLE_ENTRY_SIZE], eax
    add eax, GIGABYTE
    mov [p3_table + 3 * PAGE_TABLE_ENTRY_SIZE], eax

.done_mapping_low_page_tables:

	; Map the stack to high memory with nothing mapped directly below it
	; so that stack overflows will trigger page faults
	mov eax, p3_table_for_stack
	or eax, PAGE_TABLE_FLAGS
	mov [p4_table + LAST_PAGE_TABLE_ENTRY], eax

	mov eax, p2_table_for_stack
	or eax, PAGE_TABLE_FLAGS
	mov [p3_table_for_stack + LAST_PAGE_TABLE_ENTRY], eax

	mov eax, p1_table_for_stack
	or eax, PAGE_TABLE_FLAGS
	mov [p2_table_for_stack + LAST_PAGE_TABLE_ENTRY], eax

	mov eax, stack_bottom
	or eax, PAGE_TABLE_FLAGS
	mov [p1_table_for_stack + LAST_PAGE_TABLE_ENTRY - PAGE_TABLE_ENTRY_SIZE], eax

	mov eax, stack_bottom + PAGE_SIZE
	or eax, PAGE_TABLE_FLAGS
	mov [p1_table_for_stack + LAST_PAGE_TABLE_ENTRY], eax

    ; load p4 to cr3
    mov eax, p4_table
    mov cr3, eax

    ; enable physical address extension
    mov eax, cr4
    or eax, PHYSICAL_ADDRESS_EXPANSION
    mov cr4, eax
    ; set long mode bit
    mov ecx, EFER_MSR

    rdmsr
    or eax, EFER_LONG_MODE_NO_EXECUTE
    wrmsr

    ; enable paging
    mov eax, cr0
    or eax, PAGING_FLAG
    mov cr0, eax

    ; load 64-bit gdt
    lgdt [gdt64.pointer]

    jmp gdt64.code:long_mode_start

.map_p3_table_no_gigabyte_pages:
    ; Set up p3 table with 4 entries
    mov eax, p2_tables
    or eax, PAGE_TABLE_FLAGS
    mov [p3_table], eax
    add eax, PAGE_SIZE
    mov [p3_table + PAGE_TABLE_ENTRY_SIZE], eax
    add eax, PAGE_SIZE
    mov [p3_table + 2 * PAGE_TABLE_ENTRY_SIZE], eax
    add eax, PAGE_SIZE
    mov [p3_table + 3 * PAGE_TABLE_ENTRY_SIZE], eax

    ; Set up the p2 table for identity mapping
    mov ecx, p2_tables ; ecx is the address of the table entry to edit
    mov edx, NUM_P2_TABLES * PAGE_SIZE
    add edx, ecx ; edx is the address of the end of the last table to edit
    mov ebx, PAGE_FLAGS ; ebx is the page table entry value

.map_p2_table:
    mov [ecx], ebx
    add ebx, 0x200000
    add ecx, PAGE_TABLE_ENTRY_SIZE
    cmp ecx, edx
    jne .map_p2_table

    jmp .done_mapping_low_page_tables

stop:
    hlt
    jmp stop

; Let the kernel know where its end is
global kernel_end
section .kernelend
kernel_end:

