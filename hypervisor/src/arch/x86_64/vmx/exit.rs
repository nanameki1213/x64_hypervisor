//! VM exit ハンドラ
//!
//! exit パスではメモリ確保や長時間のロックをしないこと。

use console::println;

use super::ept::ViolationError;
use super::guest_fixed_bits;
use super::vcpu::{self, GuestRegs, Vcpu};
use super::vmcs::{self, field};
use crate::arch::x86_64::cpu::{self, CR4_VMXE};

/// CPUID 0x40000000 で返す HV のシグネチャ（ebx, ecx, edx の順に 12 バイト）
pub const HV_SIGNATURE: &[u8; 12] = b"x64-hv-thin\0";
const HV_CPUID_LEAF_BASE: u32 = 0x4000_0000;
const HV_CPUID_LEAF_MAX: u32 = 0x4000_0000;

/// 未定義の hypercall の戻り値
/// TODO(#12): Hypercall ABI を決めたらエラーコード体系を置き換える
pub const HYPERCALL_UNSUPPORTED: u64 = u64::MAX;

pub enum ExitAction {
    Resume,
    Stop(StopReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 割り込み禁止のまま HLT した（ゲストが自ら停止した）
    GuestHalted,
    /// HV が使う物理メモリにアクセスした
    HvMemoryAccess { gpa: u64 },
    /// 処理できない exit や VM-entry の失敗
    Fatal,
}

mod reason {
    pub const EXCEPTION_NMI: u32 = 0;
    pub const TRIPLE_FAULT: u32 = 2;
    pub const INIT: u32 = 3;
    pub const SIPI: u32 = 4;
    pub const CPUID: u32 = 10;
    pub const GETSEC: u32 = 11;
    pub const HLT: u32 = 12;
    pub const INVD: u32 = 13;
    pub const VMCALL: u32 = 18;
    pub const VMCLEAR: u32 = 19;
    pub const VMXON: u32 = 27;
    pub const CR_ACCESS: u32 = 28;
    pub const RDMSR: u32 = 31;
    pub const WRMSR: u32 = 32;
    pub const INVALID_GUEST_STATE: u32 = 33;
    pub const MSR_LOADING: u32 = 34;
    pub const MACHINE_CHECK: u32 = 41;
    pub const EPT_VIOLATION: u32 = 48;
    pub const EPT_MISCONFIG: u32 = 49;
    pub const INVEPT: u32 = 50;
    pub const INVVPID: u32 = 53;
    pub const XSETBV: u32 = 55;
}

const EXIT_REASON_ENTRY_FAILURE: u64 = 1 << 31;

const VECTOR_UD: u8 = 6;
const VECTOR_GP: u8 = 13;

pub fn handle(vcpu: &mut Vcpu) -> ExitAction {
    let raw_reason = vmcs::read(field::VM_EXIT_REASON);
    if raw_reason & EXIT_REASON_ENTRY_FAILURE != 0 {
        println!(
            "[vmx] VM-entry failure: reason={} qualification={:#x}",
            raw_reason & 0xffff,
            vmcs::read(field::EXIT_QUALIFICATION)
        );
        dump_guest_state(vcpu);
        return ExitAction::Stop(StopReason::Fatal);
    }

    match (raw_reason & 0xffff) as u32 {
        reason::CPUID => handle_cpuid(&mut vcpu.regs),
        reason::VMCALL => handle_vmcall(&mut vcpu.regs),
        reason::HLT => handle_hlt(),
        reason::CR_ACCESS => handle_cr_access(&mut vcpu.regs),
        reason::EPT_VIOLATION => handle_ept_violation(vcpu),
        reason::RDMSR | reason::WRMSR => {
            // MSR ビットマップで捕捉した MSR（VMX MSR）と、ビットマップの範囲外の MSR。
            // どちらもゲストには存在しないものとして扱う
            inject_hw_exception(VECTOR_GP, Some(0));
            ExitAction::Resume
        }
        reason::XSETBV => handle_xsetbv(&vcpu.regs),
        reason::INVD => {
            // INVD はキャッシュを書き戻さずに捨てるので、安全な WBINVD で代用する
            cpu::wbinvd();
            skip_instruction();
            ExitAction::Resume
        }
        // ゲストには SMX と VMX を見せていない
        reason::GETSEC | reason::VMCLEAR..=reason::VMXON | reason::INVEPT | reason::INVVPID => {
            inject_hw_exception(VECTOR_UD, None);
            ExitAction::Resume
        }
        r @ (reason::EXCEPTION_NMI
        | reason::TRIPLE_FAULT
        | reason::INIT
        | reason::SIPI
        | reason::INVALID_GUEST_STATE
        | reason::MSR_LOADING
        | reason::MACHINE_CHECK
        | reason::EPT_MISCONFIG) => {
            println!("[vmx] unhandled exit: reason={}", r);
            dump_guest_state(vcpu);
            ExitAction::Stop(StopReason::Fatal)
        }
        r => {
            println!("[vmx] unexpected exit: reason={}", r);
            dump_guest_state(vcpu);
            ExitAction::Stop(StopReason::Fatal)
        }
    }
}

/// exit を起こした命令の次から再開する
fn skip_instruction() {
    let rip = vmcs::read(field::GUEST_RIP);
    let len = vmcs::read(field::VM_EXIT_INSTRUCTION_LEN);
    vmcs::write(field::GUEST_RIP, rip + len);
}

/// 次の VM entry でゲストにハードウェア例外を注入する（RIP は進めない）
fn inject_hw_exception(vector: u8, error_code: Option<u32>) {
    const VALID: u64 = 1 << 31;
    const TYPE_HW_EXCEPTION: u64 = 3 << 8;
    const DELIVER_ERROR_CODE: u64 = 1 << 11;

    let mut info = VALID | TYPE_HW_EXCEPTION | vector as u64;
    if let Some(code) = error_code {
        info |= DELIVER_ERROR_CODE;
        vmcs::write(field::VM_ENTRY_EXCEPTION_ERROR_CODE, code as u64);
    }
    vmcs::write(field::VM_ENTRY_INTR_INFO, info);
}

/// ゲストの CPL（SS.DPL）
fn guest_cpl() -> u64 {
    (vmcs::read(field::GUEST_SS_AR_BYTES) >> 5) & 3
}

fn handle_cpuid(regs: &mut GuestRegs) -> ExitAction {
    let leaf = regs.rax as u32;
    let subleaf = regs.rcx as u32;

    let mut r = if (HV_CPUID_LEAF_BASE..=HV_CPUID_LEAF_BASE + 0xff).contains(&leaf) {
        // 下位の HV（nested 時の KVM など）の情報は見せない
        cpu::CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        }
    } else {
        cpu::cpuid(leaf, subleaf)
    };

    // VMX の制御で有効にできなかった命令は、ゲストで実行すると #UD になるので CPUID でも隠す
    let secondary = vmcs::read(field::SECONDARY_VM_EXEC_CONTROL) as u32;
    let disabled = |control: u32| secondary & control == 0;

    match leaf {
        1 => {
            r.ecx &= !(1 << 5); // VMX
            r.ecx |= 1 << 31; // hypervisor present
        }
        7 if subleaf == 0 => {
            if disabled(vcpu::CPU2_ENABLE_INVPCID) {
                r.ebx &= !(1 << 10); // INVPCID
            }
            if disabled(vcpu::CPU2_ENABLE_USER_WAIT_PAUSE) {
                r.ecx &= !(1 << 5); // WAITPKG（UMONITOR / UMWAIT / TPAUSE）
            }
            if disabled(vcpu::CPU2_ENABLE_RDTSCP) {
                r.ecx &= !(1 << 22); // RDPID
            }
        }
        0xd if subleaf == 1 && disabled(vcpu::CPU2_ENABLE_XSAVES) => {
            r.eax &= !(1 << 3); // XSAVES / XRSTORS
        }
        0x8000_0001 if disabled(vcpu::CPU2_ENABLE_RDTSCP) => {
            r.edx &= !(1 << 27); // RDTSCP
        }
        HV_CPUID_LEAF_BASE => {
            let word = |i: usize| u32::from_le_bytes(HV_SIGNATURE[i..i + 4].try_into().unwrap());
            r.eax = HV_CPUID_LEAF_MAX;
            r.ebx = word(0);
            r.ecx = word(4);
            r.edx = word(8);
        }
        _ => {}
    }

    regs.rax = r.eax as u64;
    regs.rbx = r.ebx as u64;
    regs.rcx = r.ecx as u64;
    regs.rdx = r.edx as u64;
    skip_instruction();
    ExitAction::Resume
}

fn handle_vmcall(regs: &mut GuestRegs) -> ExitAction {
    if guest_cpl() != 0 {
        inject_hw_exception(VECTOR_UD, None);
        return ExitAction::Resume;
    }
    // TODO(#12): Hypercall ABI を決めて実装する
    regs.rax = HYPERCALL_UNSUPPORTED;
    skip_instruction();
    ExitAction::Resume
}

fn handle_hlt() -> ExitAction {
    const RFLAGS_IF: u64 = 1 << 9;
    if vmcs::read(field::GUEST_RFLAGS) & RFLAGS_IF == 0 {
        return ExitAction::Stop(StopReason::GuestHalted);
    }
    // 割り込みを待つべきだが、HLT exiting を使うのは今のところテスト用ゲストだけなので、
    // 何もせずに次の命令から再開する
    skip_instruction();
    ExitAction::Resume
}

/// exit qualification のレジスタ番号（0 = RAX, 1 = RCX, ... SDM の順）に対応する値
fn gpr(regs: &GuestRegs, index: u64) -> u64 {
    match index {
        0 => regs.rax,
        1 => regs.rcx,
        2 => regs.rdx,
        3 => regs.rbx,
        4 => vmcs::read(field::GUEST_RSP),
        5 => regs.rbp,
        6 => regs.rsi,
        7 => regs.rdi,
        8 => regs.r8,
        9 => regs.r9,
        10 => regs.r10,
        11 => regs.r11,
        12 => regs.r12,
        13 => regs.r13,
        14 => regs.r14,
        _ => regs.r15,
    }
}

/// CR0 / CR4 のうちホストが持つビット（guest/host mask）をゲストが変えようとしたときの処理
fn handle_cr_access(regs: &mut GuestRegs) -> ExitAction {
    let qual = vmcs::read(field::EXIT_QUALIFICATION);
    let cr = qual & 0xf;
    let access_type = (qual >> 4) & 0x3;
    let value = gpr(regs, (qual >> 8) & 0xf);

    const MOV_TO_CR: u64 = 0;
    if access_type != MOV_TO_CR {
        // CLTS / LMSW はマスク対象のビット（TS / PE など）を変えないかぎり exit しない
        println!("[vmx] unsupported CR access: qualification={:#x}", qual);
        return ExitAction::Stop(StopReason::Fatal);
    }

    let fixed = guest_fixed_bits();
    match cr {
        0 => {
            // VMX で固定されたビット（NE など）は実際の CR0 では強制し、ゲストには書いた値を見せる
            let actual = (value | fixed.cr0_fixed1) & fixed.cr0_allowed;
            vmcs::write(field::GUEST_CR0, actual);
            vmcs::write(field::CR0_READ_SHADOW, value);
            // 実機では CR0.PG の切り替えに合わせて EFER.LMA も変わるので、代行する
            // （VM-entry の "IA-32e mode guest" は再開前に EFER.LMA に合わせる）
            const CR0_PG: u64 = 1 << 31;
            const EFER_LME: u64 = 1 << 8;
            const EFER_LMA: u64 = 1 << 10;
            let efer = vmcs::read(field::GUEST_IA32_EFER);
            let new_efer = if actual & CR0_PG != 0 && efer & EFER_LME != 0 {
                efer | EFER_LMA
            } else {
                efer & !EFER_LMA
            };
            if new_efer != efer {
                vmcs::write(field::GUEST_IA32_EFER, new_efer);
            }
        }
        4 => {
            if value & CR4_VMXE != 0 {
                // ゲストには VMX を見せていない
                inject_hw_exception(VECTOR_GP, Some(0));
                return ExitAction::Resume;
            }
            let actual = (value | fixed.cr4_fixed1 | CR4_VMXE) & fixed.cr4_allowed;
            vmcs::write(field::GUEST_CR4, actual);
            vmcs::write(field::CR4_READ_SHADOW, value);
        }
        _ => {
            println!("[vmx] unexpected CR{} access", cr);
            return ExitAction::Stop(StopReason::Fatal);
        }
    }
    skip_instruction();
    ExitAction::Resume
}

fn handle_ept_violation(vcpu: &mut Vcpu) -> ExitAction {
    let gpa = vmcs::read(field::GUEST_PHYSICAL_ADDRESS);
    match vcpu.ept.handle_violation(gpa) {
        Ok(()) => ExitAction::Resume,
        Err(ViolationError::HvMemory) => {
            println!("[vmx] guest accessed HV memory: gpa={:#x}", gpa);
            dump_guest_state(vcpu);
            ExitAction::Stop(StopReason::HvMemoryAccess { gpa })
        }
        Err(ViolationError::AlreadyMapped) => {
            println!(
                "[vmx] unexpected EPT violation on a mapped page: gpa={:#x}",
                gpa
            );
            dump_guest_state(vcpu);
            ExitAction::Stop(StopReason::Fatal)
        }
    }
}

/// XSETBV は無条件に exit する。ホストで実行して #GP にならないよう、値を検証してから実行する
fn handle_xsetbv(regs: &GuestRegs) -> ExitAction {
    let xcr = regs.rcx as u32;
    let value = (regs.rdx << 32) | (regs.rax & 0xffff_ffff);

    let supported = {
        let r = cpu::cpuid(0xd, 0);
        ((r.edx as u64) << 32) | r.eax as u64
    };
    let has = |bits: u64| value & bits == bits;
    let any = |bits: u64| value & bits != 0;

    const X87: u64 = 1 << 0;
    const SSE: u64 = 1 << 1;
    const AVX: u64 = 1 << 2;
    const MPX: u64 = 0b11 << 3;
    const AVX512: u64 = 0b111 << 5;
    const AMX: u64 = 0b11 << 17;

    let valid = xcr == 0
        && value & !supported == 0
        && has(X87)
        && (!any(AVX) || has(SSE))
        && (!any(MPX) || has(MPX))
        && (!any(AVX512) || (has(AVX512) && has(AVX)))
        && (!any(AMX) || has(AMX));
    if !valid {
        inject_hw_exception(VECTOR_GP, Some(0));
        return ExitAction::Resume;
    }

    // HV 自身は XSAVE 管理下の状態を使わないので、ゲストの値をそのまま設定してよい
    unsafe { cpu::xsetbv(xcr, value) };
    skip_instruction();
    ExitAction::Resume
}

pub fn dump_guest_state(vcpu: &Vcpu) {
    let r = |f| vmcs::vmread(f).unwrap_or(0xdead_dead_dead_dead);
    let regs = &vcpu.regs;
    println!(
        "[vmx] ---- guest state (VMCS {:#x}) ----",
        vcpu.vmcs_address()
    );
    println!(
        "  exit reason={:#x} qualification={:#x} intr_info={:#x} idt_vectoring={:#x}",
        r(field::VM_EXIT_REASON),
        r(field::EXIT_QUALIFICATION),
        r(field::VM_EXIT_INTR_INFO),
        r(field::IDT_VECTORING_INFO)
    );
    println!(
        "  RIP={:#018x} RSP={:#018x} RFLAGS={:#018x}",
        r(field::GUEST_RIP),
        r(field::GUEST_RSP),
        r(field::GUEST_RFLAGS)
    );
    println!(
        "  CR0={:#018x} CR3={:#018x} CR4={:#018x} EFER={:#018x}",
        r(field::GUEST_CR0),
        r(field::GUEST_CR3),
        r(field::GUEST_CR4),
        r(field::GUEST_IA32_EFER)
    );
    println!(
        "  CS={:#06x} (AR {:#x}) SS={:#06x} (AR {:#x}) TR={:#06x} (AR {:#x})",
        r(field::GUEST_CS_SELECTOR),
        r(field::GUEST_CS_AR_BYTES),
        r(field::GUEST_SS_SELECTOR),
        r(field::GUEST_SS_AR_BYTES),
        r(field::GUEST_TR_SELECTOR),
        r(field::GUEST_TR_AR_BYTES)
    );
    println!(
        "  guest linear={:#x} guest physical={:#x}",
        r(field::GUEST_LINEAR_ADDRESS),
        r(field::GUEST_PHYSICAL_ADDRESS)
    );
    println!(
        "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
        regs.rax, regs.rbx, regs.rcx, regs.rdx
    );
    println!(
        "  RSI={:#018x} RDI={:#018x} RBP={:#018x}",
        regs.rsi, regs.rdi, regs.rbp
    );
    println!(
        "  R8 ={:#018x} R9 ={:#018x} R10={:#018x} R11={:#018x}",
        regs.r8, regs.r9, regs.r10, regs.r11
    );
    println!(
        "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
        regs.r12, regs.r13, regs.r14, regs.r15
    );
}
