; Declare constants for the multiboot header
MULTIBOOT_2   equ 0xe85250d6 ; 'magic number' lets bootloader find the header
X86           equ 0          ; architecture enum value for x86
HEADER_LENGTH equ header_end - header_start

; Declare contants used in printing
VGA_BUFFER    equ 0xb8000
ER            equ 0x4f524f45
R_COLON       equ 0x4f3a4f52
SPACE         equ 0x4f204f20

; Miscelaneous constants
MULTIBOOT_CHECK       equ 0x36d76289
CPUID_BIT             equ 1 << 21
LONG_MODE_CPUID_BIT   equ 1 << 29
PAGE_SIZE             equ 4 * 1024
PAGE_TABLE_ENTRY_SIZE equ 8
STACK_SIZE            equ 256 * PAGE_SIZE
HUGE_PAGE_SIZE        equ 2 * 1024 * 1024
NUM_P2_TABLES         equ 4
PRESENT_WRITABLE      equ 3
PRESENT_WRITABLE_HUGE equ (0x80 + PRESENT_WRITABLE)
PAGE_TABLE_ENTRIES    equ 512
PAE_FLAG              equ 1 << 5
LONG_MODE_EFER        equ 1 << 8
EFER_MSR              equ 0xC0000080
PAGING_FLAG           equ 1 << 31
LONG_CODE_SEGMENT     equ (1<<43) + (1<<44) + (1<<47) + (1<<53)

; Error codes
NO_MULTIBOOT equ "0"
NO_CPUID     equ "1"
NO_LONG_MODE equ "2"
NO_KERNEL    equ "9"

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

    ; required end tag
    dw 0 ; type
    dw 0 ; flags
    dd 8 ; size
header_end:

global p4_table
section .bss
; Page tables
align PAGE_SIZE
p4_table:
    resb PAGE_SIZE
p3_table:
    resb PAGE_SIZE
; Allow 4 p2 tables so we can identity map 4 GB
p2_tables:
    resb NUM_P2_TABLES * PAGE_SIZE
; Stack
stack_bottom:
    resb STACK_SIZE
stack_top:

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
    mov esp, stack_top

    ; Make sure the kernel was loaded by a multiboot compliant bootloader
    cmp eax, MULTIBOOT_CHECK
    jne .no_multiboot

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
    je .no_cpuid

    ; Stash ebx to edi before calling cpuid
    mov edi, ebx

    ; Make sure long mode (64 bit mode) is supported
    mov eax, 0x80000000
    cpuid
    cmp eax, 0x80000001
    jb .no_long_mode
    mov eax, 0x80000001
    cpuid
    test edx, LONG_MODE_CPUID_BIT
    jz .no_long_mode

    ; Set up p4 table with 1 entry
    mov eax, p3_table
    or eax, PRESENT_WRITABLE
    mov [p4_table], eax

    ; Set up p3 table with 4 entries
    mov eax, p2_tables
    or eax, PRESENT_WRITABLE
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
    xor ebx, ebx ; ebx is the address of the page frame to map to

.map_p2_table:
    mov eax, ebx
    add ebx, HUGE_PAGE_SIZE
    or eax, PRESENT_WRITABLE_HUGE
    mov [ecx], eax
    add ecx, PAGE_TABLE_ENTRY_SIZE
    cmp ecx, edx
    jne .map_p2_table

    ; load p4 to cr3
    mov eax, p4_table
    mov cr3, eax
    ; enable physical address extension
    mov eax, cr4
    or eax, PAE_FLAG
    mov cr4, eax
    ; set long mode bit
    mov ecx, EFER_MSR
    rdmsr
    or eax, LONG_MODE_EFER
    wrmsr
    ; enable paging
    mov eax, cr0
    or eax, PAGING_FLAG
    mov cr0, eax

    ; load 64-bit gdt
    lgdt [gdt64.pointer]

    jmp gdt64.code:long_mode_start

.no_multiboot:
    mov al, NO_MULTIBOOT
    jmp error

.no_cpuid:
    mov al, NO_CPUID
    jmp error

.no_long_mode:
    mov al, NO_LONG_MODE
    jmp error

; Handle errors by printing an error code
error:
    mov dword [VGA_BUFFER], ER
    mov dword [VGA_BUFFER + 4], R_COLON
    mov dword [VGA_BUFFER + 8], SPACE
    mov byte  [VGA_BUFFER + 10], al
    hlt

stop:
    hlt
    jmp stop

; Let the kernel know where its end is
global kernel_end
section .kernelend
kernel_end:

