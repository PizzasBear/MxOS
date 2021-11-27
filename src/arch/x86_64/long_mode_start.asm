extern kernel_main
extern alloc_stack
extern special_pd_table

global long_mode_start

section .text
bits 64
long_mode_start:
    mov ax, 0
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    ; Gets rbx that contains the multiboot info pointer
    ; Checks whether it is zero
    pop rbx
    test rbx, rbx
    jnz .rbx_is_fine

    mov al, "4"
    jmp error
.rbx_is_fine:

    mov rax, 0x2f592f412f4b2f4f
    mov qword [0xb8000], rax

    mov rdi, rbx
    mov rsi, special_pd_table
    call alloc_stack

    mov rsp, (511<<39) + (510<<30) + (2<<21) | (0xffff << 48)

    ; .debug_rsp_sx:
    ; mov rcx, rsp
    ; or rcx, qword 0xffff << 48
    ; test rcx, qword 1<<47
    ; cmovnz rsp, rcx

    mov rdi, rbx
    mov rsi, rax
    jmp kernel_main

error:
    mov dword [0xb8000], 0x4f524f45
    mov dword [0xb8004], 0x4f3a4f52
    mov dword [0xb8008], 0x4f204f20
    mov byte  [0xb800a], al
.loop:
    hlt
    jmp .loop

