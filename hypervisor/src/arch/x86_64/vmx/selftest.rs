//! VT-x 基盤の動作確認用の最小ゲスト。
//!
//! HV と同じページテーブル・GDT・IDT を使って HV のテキスト上のコードを実行し、
//! CPUID / VMCALL / CR4 / XSETBV の扱いを確認して、結果を R12 に入れて HLT する。
//! TODO(#6): 管理 VM（Linux）を起動するようになったら起動経路から外す

use core::arch::global_asm;

use console::println;

use super::exit::StopReason;
use super::vcpu::{GuestConfig, Vcpu};
use super::{VmxInitError, alloc_zeroed_page};
use crate::arch::x86_64::cpu;

/// テストゲストが R12 に立てるビット
const CHECK_HV_SIGNATURE: u64 = 1 << 0;
const CHECK_VMX_HIDDEN: u64 = 1 << 1;
const CHECK_HYPERVISOR_BIT: u64 = 1 << 2;
const CHECK_VMCALL_UNSUPPORTED: u64 = 1 << 3;
const CHECK_CR4_VMXE_HIDDEN: u64 = 1 << 4;
/// XSAVE 非対応の CPU では実行せずに立てる
const CHECK_XSETBV: u64 = 1 << 5;
const ALL_CHECKS: u64 = CHECK_HV_SIGNATURE
    | CHECK_VMX_HIDDEN
    | CHECK_HYPERVISOR_BIT
    | CHECK_VMCALL_UNSUPPORTED
    | CHECK_CR4_VMXE_HIDDEN
    | CHECK_XSETBV;

unsafe extern "C" {
    fn vmx_selftest_guest();
}

// "x64-" をリトルエンディアンで読んだ値（exit::HV_SIGNATURE の先頭 4 バイト）
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

9:
    cli
    hlt
    jmp   9b
"#,
    options(att_syntax)
);

pub fn run() -> Result<(), VmxInitError> {
    super::enable()?;

    let stack = alloc_zeroed_page();
    let config = GuestConfig {
        rip: vmx_selftest_guest as *const () as u64,
        rsp: stack + 4096,
        cr3: cpu::read_cr3(),
        hlt_exiting: true,
    };
    let mut vcpu = Vcpu::new(&config);
    let reason = vcpu.run();

    let result = vcpu.regs.r12;
    if reason != StopReason::GuestHalted || result != ALL_CHECKS {
        panic!(
            "VMX self test failed: stop reason={:?}, checks={:#x} (expected {:#x})",
            reason, result, ALL_CHECKS
        );
    }
    println!("[vmx] self test passed (checks={:#x})", result);
    Ok(())
}
