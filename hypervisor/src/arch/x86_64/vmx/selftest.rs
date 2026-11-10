//! VT-x / EPT の動作確認。
//!
//! 1. 管理 VM 用の EPT を作り、HV の物理メモリがマップされていないことを確認する
//! 2. 最小のゲストを実行する。ゲストは HV と同じページテーブル・GDT・IDT を使って HV のテキスト上の
//!    コードを実行し、CPUID / VMCALL / CR4 / XSETBV / EPT の遅延マップの扱いを確認して結果を R12 に
//!    入れ、最後に HV のヒープを読んで EPT violation で止められる
//!
//! ゲストが HV のイメージを使うため、テストゲストの EPT では HV のヒープだけを隠す。
//!
//! 数ミリ秒で終わり、実機でも VT-x / EPT の前提が成り立つかを管理 VM の起動前に確かめられるので、
//! 起動のたびに実行している。CI（nested VMX なし）ではここまで到達しない。

use core::arch::global_asm;

use console::println;

use super::ept::{self, Ept};
use super::exit::StopReason;
use super::vcpu::{GuestConfig, Vcpu};
use crate::mm::{MemoryLayout, PAGE_SIZE_1G};

/// テストゲストが R12 に立てるビット
const CHECK_HV_SIGNATURE: u64 = 1 << 0;
const CHECK_VMX_HIDDEN: u64 = 1 << 1;
const CHECK_HYPERVISOR_BIT: u64 = 1 << 2;
const CHECK_VMCALL_UNSUPPORTED: u64 = 1 << 3;
const CHECK_CR4_VMXE_HIDDEN: u64 = 1 << 4;
/// XSAVE 非対応の CPU では実行せずに立てる
const CHECK_XSETBV: u64 = 1 << 5;
const CHECK_LAZY_MAP: u64 = 1 << 6;
const ALL_CHECKS: u64 = CHECK_HV_SIGNATURE
    | CHECK_VMX_HIDDEN
    | CHECK_HYPERVISOR_BIT
    | CHECK_VMCALL_UNSUPPORTED
    | CHECK_CR4_VMXE_HIDDEN
    | CHECK_XSETBV
    | CHECK_LAZY_MAP;

const GUEST_STACK_SIZE: usize = 0x2000;

#[repr(C, align(16))]
struct GuestStack([u8; GUEST_STACK_SIZE]);

/// テストゲストのスタック。HV のヒープはゲストから見えないので、イメージ内に置く
static mut GUEST_STACK: GuestStack = GuestStack([0; GUEST_STACK_SIZE]);

unsafe extern "C" {
    fn vmx_selftest_guest();
}

// 入力: R13 = 起動時にマップしていないアドレス、R14 = HV のヒープのアドレス
// "x64-" をリトルエンディアンで読んだ値は exit::HV_SIGNATURE の先頭 4 バイト
global_asm!(
    r#"
.section .text
.global vmx_selftest_guest
vmx_selftest_guest:
    xor   %r12d, %r12d

    mov   $0x40000000, %eax
    xor   %ecx, %ecx
    cpuid
    cmp   $0x2d343678, %ebx
    jne   1f
    or    $0x1, %r12
1:
    mov   $1, %eax
    xor   %ecx, %ecx
    cpuid
    bt    $5, %ecx
    jc    2f
    or    $0x2, %r12
2:
    bt    $31, %ecx
    jnc   3f
    or    $0x4, %r12
3:
    xor   %eax, %eax
    vmcall
    cmp   $-1, %rax
    jne   4f
    or    $0x8, %r12
4:
    mov   %cr4, %rax
    bt    $13, %rax
    jc    5f
    or    $0x10, %r12
    # VMXE を 0 のまま書き戻しても、実際の CR4.VMXE は維持される
    mov   %rax, %cr4
5:
    # CR4.OSXSAVE を立て、現在の XCR0 を XSETBV で書き戻す
    mov   $1, %eax
    xor   %ecx, %ecx
    cpuid
    bt    $26, %ecx
    jnc   6f
    mov   %cr4, %rax
    bts   $18, %rax
    mov   %rax, %cr4
    xor   %ecx, %ecx
    xgetbv
    xor   %ecx, %ecx
    xsetbv
6:
    or    $0x20, %r12

    # 起動時にマップしていないアドレスは、EPT violation を契機に遅延マップされて読める
    mov   (%r13), %rax
    or    $0x40, %r12

    # HV のヒープは EPT でマップされていないので、ここで止められる
    mov   (%r14), %rax

9:
    cli
    hlt
    jmp   9b
"#,
    options(att_syntax)
);

/// 管理 VM 用の EPT で、HV のメモリが隠れていて、それ以外が恒等マップされていることを確認する
fn check_management_vm_ept(layout: &MemoryLayout) {
    let reserved = layout.reserved();
    let is_reserved = |gpa: u64| reserved.iter().any(|r| r.contains(&gpa));

    let mut ept = Ept::new(&reserved).expect("EPT is not supported");
    let range = ept::initial_identity_range(layout);
    ept.map_identity(range.clone());

    for r in &reserved {
        for gpa in [r.start, r.start + 0x1000, r.end - 1] {
            assert!(
                ept.translate(gpa).is_none(),
                "HV memory {:#x} is mapped in the management VM's EPT",
                gpa
            );
        }
        // HV のメモリの直前・直後は恒等マップされている
        for gpa in [r.start.wrapping_sub(1), r.end] {
            if gpa < range.end && !is_reserved(gpa) {
                assert_eq!(
                    ept.translate(gpa),
                    Some(gpa),
                    "{:#x} is not identity mapped",
                    gpa
                );
            }
        }
    }
    // 低位メモリ、ROM 領域、LAPIC、4GiB 直下
    for gpa in [0, 0x9_f000, 0xf_0000, 0xfee0_0000, 4 * PAGE_SIZE_1G - 1] {
        assert_eq!(
            ept.translate(gpa),
            Some(gpa),
            "{:#x} is not identity mapped",
            gpa
        );
    }

    let stats = ept.stats();
    println!(
        "[ept] management VM: identity {:#x}-{:#x}, 1G={} 2M={} 4K={} tables={}",
        range.start, range.end, stats.pages_1g, stats.pages_2m, stats.pages_4k, stats.tables
    );
}

/// VMX を有効にした（`vmx::enable`）後に呼ぶこと。失敗したら panic する
pub fn run(layout: &MemoryLayout) {
    check_management_vm_ept(layout);

    let mut ept = Ept::new(core::slice::from_ref(&layout.heap)).expect("EPT is not supported");
    let initial = ept::initial_identity_range(layout);
    ept.map_identity(initial.clone());
    let lazy_gpa = initial.end + PAGE_SIZE_1G;
    assert!(ept.translate(lazy_gpa).is_none());

    let config = GuestConfig::mirror_host(
        vmx_selftest_guest as *const () as u64,
        (&raw const GUEST_STACK) as u64 + GUEST_STACK_SIZE as u64,
        true,
    );
    let mut vcpu = Vcpu::new(&config, ept);
    vcpu.regs.r13 = lazy_gpa;
    vcpu.regs.r14 = layout.heap.start;
    let reason = vcpu.run();

    let checks = vcpu.regs.r12;
    let expected = StopReason::HvMemoryAccess {
        gpa: layout.heap.start,
    };
    if reason != expected || checks != ALL_CHECKS {
        panic!(
            "VMX self test failed: stop reason={:?} (expected {:?}), checks={:#x} (expected {:#x})",
            reason, expected, checks, ALL_CHECKS
        );
    }
    assert_eq!(vcpu.ept.translate(lazy_gpa), Some(lazy_gpa));
    println!("[vmx] self test passed (checks={:#x})", checks);
}
