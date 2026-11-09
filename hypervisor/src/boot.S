.set PTE_P,   1 << 0
.set PTE_W,   1 << 1
.set PTE_PS,  1 << 7

.set PAGE_2M_SIZE,    0x200000
.set PAGE_2M_MASK,    PAGE_2M_SIZE - 1
.set PAGE_1G_SIZE,    0x40000000
.set BOOT_STACK_SIZE, 0x10000

.section .text.boot, "ax"
.code64
.global _start
_start:
    cli # interrupt disable
    cld

    # rcx = boot_info (win64 ABI の第1引数)。main に渡すため r12 に退避する
    mov   %rcx, %r12

    # rsi = _start のロード先物理アドレス
    # r8  = _start のリンク時仮想アドレス
    lea   _start(%rip), %rsi
    movabs $_start, %r8

    # 2MiB ページでマッピングするので、ロード先が 2MiB アラインでなければ停止
    test  $PAGE_2M_MASK, %rsi
    jnz   .Lhalt

    # 恒等マッピングに 1GiB ページを使うので、未対応なら停止
    mov   $0x80000001, %eax
    cpuid
    bt    $26, %edx # Page1GB
    jnc   .Lhalt

    # ページテーブル 4 枚をゼロクリア
    lea   boot_pml4(%rip), %rdi
    mov   $(4 * 512), %ecx
    xor   %eax, %eax
    rep stosq

    # 恒等マッピング:
    # 物理メモリ全体をヒープとして登録するため、下位 512GiB を 1GiB ページで恒等マップする
    # TODO(#9): HV のアドレス空間（直接マップ）を設計したら置き換える
    lea   boot_pdpt_low(%rip), %rax
    mov   $512, %ecx
    mov   $(PTE_P | PTE_W | PTE_PS), %rdx
1:  mov   %rdx, (%rax)
    add   $8, %rax
    add   $PAGE_1G_SIZE, %rdx
    dec   %ecx
    jnz   1b

    # 上位マッピング:
    # pd_high[(virt >> 21) & 0x1ff] 以降を phys から 2MiB ずつ埋める
    mov   %r8, %rdi
    shr   $21, %rdi
    and   $0x1ff, %rdi
    lea   boot_pd_high(%rip), %rax
    lea   (%rax, %rdi, 8), %rax
    mov   $512, %ecx
    sub   %edi, %ecx
    mov   %rsi, %rdx
    or    $(PTE_P | PTE_W | PTE_PS), %rdx
2:  mov   %rdx, (%rax)
    add   $8, %rax
    add   $PAGE_2M_SIZE, %rdx
    dec   %ecx
    jnz   2b

    # pdpt_high[(virt >> 30) & 0x1ff] = pd_high
    mov   %r8, %rdi
    shr   $30, %rdi
    and   $0x1ff, %rdi
    lea   boot_pd_high(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    lea   boot_pdpt_high(%rip), %rdx
    mov   %rax, (%rdx, %rdi, 8)

    # pml4[0] = pdpt_low
    lea   boot_pml4(%rip), %rcx
    lea   boot_pdpt_low(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    mov   %rax, (%rcx)

    # pml4[(virt >> 39) & 0x1ff] = pdpt_high
    mov   %r8, %rdi
    shr   $39, %rdi
    and   $0x1ff, %rdi
    lea   boot_pdpt_high(%rip), %rax
    or    $(PTE_P | PTE_W), %rax
    mov   %rax, (%rcx, %rdi, 8)

    mov   %rcx, %cr3

    # 上位アドレスのスタックに切り替え、上位アドレスの main を呼ぶ
    movabs $boot_stack_top, %rsp
    sub   $32, %rsp # win64 ABI のシャドウスペース
    mov   %r12, %rcx
    movabs $main, %rax
    call  *%rax

.Lhalt:
    hlt
    jmp   .Lhalt

.section .bss
.balign 4096
boot_pml4:      .skip 4096
boot_pdpt_low:  .skip 4096
boot_pdpt_high: .skip 4096
boot_pd_high:   .skip 4096

.balign 16
boot_stack:     .skip BOOT_STACK_SIZE
boot_stack_top:
