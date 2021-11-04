section .multiboot_header

MAGIC equ 0xe85250d6
ARCH equ 0x0

header_start:
    dd MAGIC                     ; magic number (multiboot 2)
    dd ARCH                      ; architecture 0 (protected mode i386)
    dd header_end - header_start ; header length
    ; checksum
    dd (1 << 32) - (MAGIC + ARCH + (header_end - header_start))

    ; ; framebuffer tag
    ; align 8 ; tags should be 64-bit aligned
    ; dw 5    ; type
    ; dw 0    ; flags
    ; dd 20   ; size
    ; dd 0    ; width
    ; dd 0    ; height
    ; dd 32   ; depth (bits per pixel)

    ; required end tag
    align 8 ; tags should be 64-bit aligned
    dw 0    ; type
    dw 0    ; flags
    dd 8    ; size
header_end:
