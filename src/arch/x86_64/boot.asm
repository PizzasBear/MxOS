extern long_mode_start

global start
global special_pd_table

section .text
bits 32
start:
    mov esp, stack_top
    push dword 0
    push ebx

    call check_multiboot

    call check_cpuid
    call check_long_mode

    call set_up_page_tables
    call enable_paging

    lgdt [gdt64.pointer]

    jmp gdt64.code:long_mode_start

    mov al, "3"
    jmp error

error:
    mov dword [0xb8000], 0x4f524f45
    mov dword [0xb8004], 0x4f3a4f52
    mov dword [0xb8008], 0x4f204f20
    mov byte  [0xb800a], al
.loop:
    hlt
    jmp .loop

check_multiboot:
    cmp eax, 0x36d76289
    jne .no_multiboot
    ret
.no_multiboot:
    mov al, "0"
    jmp error

check_cpuid:
    pushfd
    pop eax

    ; Save EAX to ECX
    mov ecx, eax

    ; Flip the ID bit
    xor eax, 1 << 21

    push eax
    popfd

    ; Copy FLAGS back to EAX (with the flipped bit if CPUID is supported)
    pushfd
    pop eax

    ; Restore FLAGS from the old version stored in ECX (i.e. flipping the ID bit
    ; back if it was ever flipped).
    push ecx
    popfd

    xor eax, ecx
    jz .no_cpuid
    ret
.no_cpuid:
    mov al, "1"
    jmp error

check_long_mode:
    mov eax, 0x80000000
    cpuid
    cmp eax, 0x80000001
    jb .no_long_mode
    mov eax, 0x80000001
    cpuid
    test edx, 1 << 29
    jz .no_long_mode
    ret
.no_long_mode:
    mov al, "2"
    jmp error

set_up_page_tables:
    mov eax, pdp0_table
    or eax, 3
    mov [pml4_table], eax

    mov eax, pdp511_table
    or eax, 3
    mov [pml4_table + (511*8)], eax

    mov dword [pdp0_table], 0x83
    mov dword [pdp0_table+8], 0x40000083
    mov dword [pdp0_table+16], 0x80000083
    mov dword [pdp0_table+24], 0xC0000083

    ; special page table
    mov eax, special_pd_table
    or eax, 3
    mov [pdp511_table + (510*8)], eax

    ret
;     mov eax, pdp_table
;     or eax, 0b11
;     mov [pml4_table], eax
; 
;     mov eax, pd_table
;     or eax, 0b11
;     mov [pdp_table], eax
; 
;     mov ecx, 0
; .map_pd_table:
;     mov eax, 0x20000
;     ; eax *= ecx
;     mul ecx
;     or eax, 0x83
;     mov [pd_table + ecx * 8], eax
; 
;     inc ecx
;     cmp ecx, 512
;     jne .map_pd_table
; 
;     ret

enable_paging:
    ; load P4 to cr3 register (cpu uses this to access the P4 table)
    mov eax, pml4_table
    mov cr3, eax

    ; enable PAE-flag in cr4 (Physical Address Extension)
    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax

    ; set the long mode bit in the EFER MSR (model specific register)
    mov ecx, 0xC0000080
    rdmsr
    or eax, 1 << 8
    wrmsr

    ; enable paging in the cr0 register
    mov eax, cr0
    or eax, 1 << 31
    mov cr0, eax
    ret

section .bss
align 4096
pml4_table: ; Page-Map Level-4 Table
    resb 4096
pdp0_table: ; Page-Directory Pointer Table
    resb 4096
pdp511_table: ; Page-Directory Pointer Table
    resb 4096
special_pd_table: ; Page-Directory Table
    resb 4096
stack_bottom:
    resb 4*4096
stack_top:

section .rodata
gdt64:
    dq 0
.code equ $ - gdt64
    dq (1<<43) | (1<<44) | (1<<47) | (1<<53) ; code segment
.pointer:
    dw $ - gdt64 - 1
    dq gdt64
