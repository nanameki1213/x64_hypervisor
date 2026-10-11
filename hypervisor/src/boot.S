.set PTE_P,   1 << 0
.set PTE_W,   1 << 1
.set PTE_PS,  1 << 7

.section .text.boot, "ax"
.code64
.global _start
_start:
    cli # interrupt disable

    lea   boot_pml4(%rip), %rax
    mov   $(4 * 512), %ecx
1:  movq  $0, %rax
    add   $8, %rax
    dec   %ecx
    jnz   1b

    lea   boot_pd(%rip), %rax
    mov   $(PTE_P | PTE_W | PTE_PS), %edx
    mov   $512, %ecx
2:  mov   %rdx, (%rax)
    add   $8, %rax
    add   $0x200000, %rdx
    dec   %ecx
    jnz   2b

    lea   boot_pd(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    lea   boot_pdpt_low(%rip), %rdx
    mov   %rax, (%rdx)
    lea   boot_pdpt_high(%rip), %rdx
    mov   %rax, 510 * 8(%rdx)

    lea   boot_pml4(%rip), %rcx
    lea   boot_pdpt_low(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    mov   %rax, (%rcx)
    lea   boot_pdpt_high(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    mov   %rax, 511 * 8(%rcx)

    mov   %rcx, %cr3
    call  main

.section .bss
.balign 4096
boot_pml4:      .skip 4096
boot_pdpt_low:  .skip 4096
boot_pdpt_high: .skip 4096
boot_pd:        .skip 4096
