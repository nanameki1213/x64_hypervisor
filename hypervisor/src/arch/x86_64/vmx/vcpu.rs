//! vCPU（VMCS 1 つ分）の初期化と実行ループ

use core::arch::global_asm;

use spin::Mutex;

use console::println;

use super::ept::Ept;
use super::exit::{self, ExitAction, StopReason};
use super::vmcs::{self, field};
use super::{alloc_zeroed_page, guest_fixed_bits, vmcs_revision_id};
use crate::arch::x86_64::cpu::{self, CR4_VMXE};
use crate::arch::x86_64::gdt;

/// VMCS に含まれない汎用レジスタ。`vmx_run` / `vmx_exit_entry` のオフセットと一致させること
#[repr(C)]
#[derive(Default, Debug)]
pub struct GuestRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// セグメントレジスタの状態（VMCS の形式）
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    pub access_rights: u32,
}

impl Segment {
    pub const UNUSABLE: Segment = Segment {
        selector: 0,
        base: 0,
        limit: 0,
        access_rights: AR_UNUSABLE,
    };

    pub const fn flat(selector: u16, access_rights: u32) -> Segment {
        Segment {
            selector,
            base: 0,
            limit: 0xffff_ffff,
            access_rights,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DescriptorTable {
    pub base: u64,
    pub limit: u16,
}

/// ゲストの初期状態（汎用レジスタは `Vcpu::regs` で設定する）
#[derive(Clone, Debug)]
pub struct GuestConfig {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    /// ゲストに見せる値。VMX で固定されたビットは実際のレジスタでだけ強制する
    pub cr0: u64,
    pub cr3: u64,
    /// ゲストに見せる値。CR4.VMXE は実際のレジスタでだけ立てる
    pub cr4: u64,
    pub efer: u64,
    pub cs: Segment,
    pub ss: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub tr: Segment,
    pub ldtr: Segment,
    pub gdtr: DescriptorTable,
    pub idtr: DescriptorTable,
    /// HLT で VM exit するか。管理 VM では物理 CPU をそのまま止めたいので false にする
    pub hlt_exiting: bool,
}

impl GuestConfig {
    /// HV と同じ GDT / IDT / TSS / ページテーブルを使う 64bit モードのゲスト（セルフテスト用）
    pub fn mirror_host(rip: u64, rsp: u64, hlt_exiting: bool) -> Self {
        let gdtr = cpu::sgdt();
        let idtr = cpu::sidt();
        let data = Segment::flat(cpu::read_ds(), AR_DATA);
        GuestConfig {
            rip,
            rsp,
            rflags: RFLAGS_RESERVED,
            cr0: cpu::read_cr0(),
            cr3: cpu::read_cr3(),
            cr4: cpu::read_cr4() & !CR4_VMXE,
            efer: cpu::rdmsr(cpu::IA32_EFER),
            cs: Segment::flat(cpu::read_cs(), AR_CODE64),
            ss: data,
            ds: data,
            es: data,
            fs: Segment::UNUSABLE,
            gs: Segment::UNUSABLE,
            tr: Segment {
                selector: cpu::read_tr(),
                base: gdt::tss_base(),
                limit: gdt::tss_limit() as u32,
                access_rights: AR_TSS64_BUSY,
            },
            ldtr: Segment::UNUSABLE,
            gdtr: DescriptorTable {
                base: gdtr.base,
                limit: gdtr.limit,
            },
            idtr: DescriptorTable {
                base: idtr.base,
                limit: idtr.limit,
            },
            hlt_exiting,
        }
    }

    /// ページングが有効な 64bit モードのゲスト。割り込みは禁止、IDT は空。
    /// `gdtr` の `code_selector` / `data_selector` に 64bit コード / データのディスクリプタがあること
    pub fn long_mode(
        rip: u64,
        rsp: u64,
        cr3: u64,
        gdtr: DescriptorTable,
        code_selector: u16,
        data_selector: u16,
    ) -> Self {
        const CR0_PE: u64 = 1 << 0;
        const CR0_ET: u64 = 1 << 4;
        const CR0_NE: u64 = 1 << 5;
        const CR0_PG: u64 = 1 << 31;
        const CR4_PAE: u64 = 1 << 5;
        const EFER_LME: u64 = 1 << 8;
        const EFER_LMA: u64 = 1 << 10;

        let data = Segment::flat(data_selector, AR_DATA);
        GuestConfig {
            rip,
            rsp,
            rflags: RFLAGS_RESERVED,
            cr0: CR0_PE | CR0_ET | CR0_NE | CR0_PG,
            cr3,
            cr4: CR4_PAE,
            efer: EFER_LME | EFER_LMA,
            cs: Segment::flat(code_selector, AR_CODE64),
            ss: data,
            ds: data,
            es: data,
            fs: data,
            gs: data,
            // ゲストは TR を使わないが、VM entry には有効な 64bit TSS が必要
            tr: Segment {
                selector: 0,
                base: 0,
                limit: 0x67,
                access_rights: AR_TSS64_BUSY,
            },
            ldtr: Segment::UNUSABLE,
            gdtr,
            idtr: DescriptorTable { base: 0, limit: 0 },
            hlt_exiting: false,
        }
    }
}

pub struct Vcpu {
    pub regs: GuestRegs,
    /// VM の EPT。同じ VM の vCPU で共有する
    pub ept: &'static Mutex<Ept>,
    vmcs: u64,
    launched: bool,
}

// pin-based
// （なし: 外部割り込みと NMI はゲストに直接届ける）
// primary processor-based
const CPU_HLT_EXITING: u32 = 1 << 7;
const CPU_USE_MSR_BITMAPS: u32 = 1 << 28;
const CPU_ACTIVATE_SECONDARY: u32 = 1 << 31;
// secondary processor-based
const CPU2_ENABLE_EPT: u32 = 1 << 1;
const CPU2_UNRESTRICTED_GUEST: u32 = 1 << 7;
// secondary processor-based（対応していれば有効にする。無効だとゲストで #UD になるので、
// 有効にできなかったものは CPUID でも隠す: exit::handle_cpuid）
pub(super) const CPU2_ENABLE_RDTSCP: u32 = 1 << 3;
pub(super) const CPU2_ENABLE_INVPCID: u32 = 1 << 12;
pub(super) const CPU2_ENABLE_XSAVES: u32 = 1 << 20;
pub(super) const CPU2_ENABLE_USER_WAIT_PAUSE: u32 = 1 << 26;
// VM-exit
const EXIT_HOST_ADDR_SPACE_SIZE: u32 = 1 << 9;
const EXIT_SAVE_IA32_PAT: u32 = 1 << 18;
const EXIT_LOAD_IA32_PAT: u32 = 1 << 19;
const EXIT_SAVE_IA32_EFER: u32 = 1 << 20;
const EXIT_LOAD_IA32_EFER: u32 = 1 << 21;
// VM-entry
const ENTRY_IA32E_MODE_GUEST: u32 = 1 << 9;
const ENTRY_LOAD_IA32_PAT: u32 = 1 << 14;
const ENTRY_LOAD_IA32_EFER: u32 = 1 << 15;

// セグメントの access rights（VMCS の形式）
const AR_CODE64: u32 = 0xa09b; // type=0xb, S, DPL0, P, L, G
const AR_DATA: u32 = 0xc093; // type=0x3, S, DPL0, P, D/B, G
const AR_TSS64_BUSY: u32 = 0x008b; // type=0xb, P
const AR_UNUSABLE: u32 = 1 << 16;

const RFLAGS_RESERVED: u64 = 1 << 1;

unsafe extern "C" {
    /// ゲストを実行し、VM exit したら戻る。VMLAUNCH / VMRESUME 自体が失敗したら 1 を返す
    fn vmx_run(regs: *mut GuestRegs, launched: u64) -> u64;
    fn vmx_exit_entry();
}

global_asm!(
    r#"
.set GUEST_RAX, 0
.set GUEST_RBX, 8
.set GUEST_RCX, 16
.set GUEST_RDX, 24
.set GUEST_RSI, 32
.set GUEST_RDI, 40
.set GUEST_RBP, 48
.set GUEST_R8,  56
.set GUEST_R9,  64
.set GUEST_R10, 72
.set GUEST_R11, 80
.set GUEST_R12, 88
.set GUEST_R13, 96
.set GUEST_R14, 104
.set GUEST_R15, 112
.set VMCS_HOST_RSP, 0x6c14

.section .text
.global vmx_run
vmx_run:
    push  %rbp
    push  %rbx
    push  %r12
    push  %r13
    push  %r14
    push  %r15
    push  %rdi # regs。VM exit 時は (%rsp) から取り出す

    # VM exit 時にこのスタック位置へ戻ってくるようにする
    mov   $VMCS_HOST_RSP, %rax
    vmwrite %rsp, %rax
    jbe   .Lvmx_fail

    # mov はフラグを変えないので、ゲストのレジスタを読み込む前に判定しておく
    test  %rsi, %rsi
    mov   GUEST_RAX(%rdi), %rax
    mov   GUEST_RBX(%rdi), %rbx
    mov   GUEST_RCX(%rdi), %rcx
    mov   GUEST_RDX(%rdi), %rdx
    mov   GUEST_RSI(%rdi), %rsi
    mov   GUEST_RBP(%rdi), %rbp
    mov   GUEST_R8(%rdi),  %r8
    mov   GUEST_R9(%rdi),  %r9
    mov   GUEST_R10(%rdi), %r10
    mov   GUEST_R11(%rdi), %r11
    mov   GUEST_R12(%rdi), %r12
    mov   GUEST_R13(%rdi), %r13
    mov   GUEST_R14(%rdi), %r14
    mov   GUEST_R15(%rdi), %r15
    mov   GUEST_RDI(%rdi), %rdi
    jnz   1f
    vmlaunch
    jmp   .Lvmx_fail
1:  vmresume

.Lvmx_fail:
    pop   %rdi
    pop   %r15
    pop   %r14
    pop   %r13
    pop   %r12
    pop   %rbx
    pop   %rbp
    mov   $1, %eax
    ret

.global vmx_exit_entry
vmx_exit_entry:
    push  %rdi
    mov   8(%rsp), %rdi
    mov   %rax, GUEST_RAX(%rdi)
    mov   %rbx, GUEST_RBX(%rdi)
    mov   %rcx, GUEST_RCX(%rdi)
    mov   %rdx, GUEST_RDX(%rdi)
    mov   %rsi, GUEST_RSI(%rdi)
    mov   %rbp, GUEST_RBP(%rdi)
    mov   %r8,  GUEST_R8(%rdi)
    mov   %r9,  GUEST_R9(%rdi)
    mov   %r10, GUEST_R10(%rdi)
    mov   %r11, GUEST_R11(%rdi)
    mov   %r12, GUEST_R12(%rdi)
    mov   %r13, GUEST_R13(%rdi)
    mov   %r14, GUEST_R14(%rdi)
    mov   %r15, GUEST_R15(%rdi)
    pop   %rax
    mov   %rax, GUEST_RDI(%rdi)
    pop   %rdi
    pop   %r15
    pop   %r14
    pop   %r13
    pop   %r12
    pop   %rbx
    pop   %rbp
    xor   %eax, %eax
    ret
"#,
    options(att_syntax)
);

/// capability MSR に従って制御フィールドの値を決める。
/// `required` のビットが許可されていなければ panic し、`optional` は許可されているものだけ立てる
fn adjust_controls(name: &str, msr: u32, required: u32, optional: u32) -> u64 {
    let cap = cpu::rdmsr(msr);
    let must_be_one = cap as u32;
    let may_be_one = (cap >> 32) as u32;
    if required & !may_be_one != 0 {
        panic!(
            "VMX {} control {:#x} is not supported (allowed {:#x})",
            name, required, may_be_one
        );
    }
    ((required | (optional & may_be_one) | must_be_one) & may_be_one) as u64
}

/// VMX MSR を読み書きしたら VM exit するよう MSR ビットマップを作る
fn build_msr_bitmap() -> u64 {
    let bitmap = alloc_zeroed_page() as *mut u8;
    let trap = |msr: u32| {
        // read-low: 0x000, write-low: 0x800（0x0000_0000〜0x0000_1fff）
        assert!(msr < 0x2000);
        let byte = (msr / 8) as usize;
        let bit = 1u8 << (msr % 8);
        unsafe {
            *bitmap.add(byte) |= bit;
            *bitmap.add(0x800 + byte) |= bit;
        }
    };
    for msr in cpu::IA32_VMX_MSR_FIRST..=cpu::IA32_VMX_MSR_LAST {
        trap(msr);
    }
    bitmap as u64
}

/// unrestricted guest ではゲストが CR0.PG を切り替えて IA-32e モードを出入りできるが、
/// VM-entry の "IA-32e mode guest" は自動では更新されないので、EFER.LMA に合わせる
fn sync_ia32e_mode() {
    const EFER_LMA: u64 = 1 << 10;
    let entry = vmcs::read(field::VM_ENTRY_CONTROLS);
    let lma = vmcs::read(field::GUEST_IA32_EFER) & EFER_LMA != 0;
    let want = if lma {
        entry | ENTRY_IA32E_MODE_GUEST as u64
    } else {
        entry & !(ENTRY_IA32E_MODE_GUEST as u64)
    };
    if want != entry {
        vmcs::write(field::VM_ENTRY_CONTROLS, want);
    }
}

impl Vcpu {
    /// VMCS を確保してカレントにし、64bit モードのゲストとして初期化する
    pub fn new(config: &GuestConfig, ept: &'static Mutex<Ept>) -> Self {
        let vmcs_region = alloc_zeroed_page();
        unsafe {
            *(vmcs_region as *mut u32) = vmcs_revision_id();
            vmcs::vmclear(vmcs_region).expect("vmclear failed");
            vmcs::vmptrld(vmcs_region).expect("vmptrld failed");
        }

        let vcpu = Vcpu {
            regs: GuestRegs::default(),
            ept,
            vmcs: vmcs_region,
            launched: false,
        };
        vcpu.setup_controls(config);
        vcpu.setup_host_state();
        vcpu.setup_guest_state(config);
        sync_ia32e_mode();
        vcpu
    }

    fn setup_controls(&self, config: &GuestConfig) {
        let basic = cpu::rdmsr(cpu::IA32_VMX_BASIC);
        let use_true = basic & (1 << 55) != 0;
        let pick = |true_msr, msr| if use_true { true_msr } else { msr };

        let pin = adjust_controls(
            "pin-based",
            pick(
                cpu::IA32_VMX_TRUE_PINBASED_CTLS,
                cpu::IA32_VMX_PINBASED_CTLS,
            ),
            0,
            0,
        );
        let mut primary_required = CPU_USE_MSR_BITMAPS | CPU_ACTIVATE_SECONDARY;
        if config.hlt_exiting {
            primary_required |= CPU_HLT_EXITING;
        }
        let primary = adjust_controls(
            "primary processor-based",
            pick(
                cpu::IA32_VMX_TRUE_PROCBASED_CTLS,
                cpu::IA32_VMX_PROCBASED_CTLS,
            ),
            primary_required,
            0,
        );
        let secondary = adjust_controls(
            "secondary processor-based",
            cpu::IA32_VMX_PROCBASED_CTLS2,
            // unrestricted guest は AP をリアルモードから起動するのに必要（#8）
            CPU2_ENABLE_EPT | CPU2_UNRESTRICTED_GUEST,
            CPU2_ENABLE_RDTSCP
                | CPU2_ENABLE_INVPCID
                | CPU2_ENABLE_XSAVES
                | CPU2_ENABLE_USER_WAIT_PAUSE,
        );
        let exit = adjust_controls(
            "VM-exit",
            pick(cpu::IA32_VMX_TRUE_EXIT_CTLS, cpu::IA32_VMX_EXIT_CTLS),
            EXIT_HOST_ADDR_SPACE_SIZE
                | EXIT_SAVE_IA32_PAT
                | EXIT_LOAD_IA32_PAT
                | EXIT_SAVE_IA32_EFER
                | EXIT_LOAD_IA32_EFER,
            0,
        );
        let entry = adjust_controls(
            "VM-entry",
            pick(cpu::IA32_VMX_TRUE_ENTRY_CTLS, cpu::IA32_VMX_ENTRY_CTLS),
            ENTRY_IA32E_MODE_GUEST | ENTRY_LOAD_IA32_PAT | ENTRY_LOAD_IA32_EFER,
            0,
        );

        vmcs::write(field::PIN_BASED_VM_EXEC_CONTROL, pin);
        vmcs::write(field::CPU_BASED_VM_EXEC_CONTROL, primary);
        vmcs::write(field::SECONDARY_VM_EXEC_CONTROL, secondary);
        vmcs::write(field::VM_EXIT_CONTROLS, exit);
        vmcs::write(field::VM_ENTRY_CONTROLS, entry);

        vmcs::write(field::EXCEPTION_BITMAP, 0);
        vmcs::write(field::CR3_TARGET_COUNT, 0);
        vmcs::write(field::VM_EXIT_MSR_STORE_COUNT, 0);
        vmcs::write(field::VM_EXIT_MSR_LOAD_COUNT, 0);
        vmcs::write(field::VM_ENTRY_MSR_LOAD_COUNT, 0);
        vmcs::write(field::VM_ENTRY_INTR_INFO, 0);
        vmcs::write(field::MSR_BITMAP, build_msr_bitmap());
        vmcs::write(field::EPT_POINTER, self.ept.lock().eptp());

        // CR0 / CR4 の VMX で固定されたビットはホストが持ち、ゲストの書き込みを捕捉する。
        // CR4.VMXE はゲストからは 0 に見せる
        let fixed = guest_fixed_bits();
        vmcs::write(
            field::CR0_GUEST_HOST_MASK,
            fixed.cr0_fixed1 | !fixed.cr0_allowed,
        );
        vmcs::write(
            field::CR4_GUEST_HOST_MASK,
            fixed.cr4_fixed1 | !fixed.cr4_allowed | CR4_VMXE,
        );
    }

    fn setup_host_state(&self) {
        let gdtr = cpu::sgdt();
        let idtr = cpu::sidt();
        let tr = cpu::read_tr();

        vmcs::write(field::HOST_CR0, cpu::read_cr0());
        vmcs::write(field::HOST_CR3, cpu::read_cr3());
        vmcs::write(field::HOST_CR4, cpu::read_cr4());

        // ホストのセレクタは RPL / TI が 0 でなければならない
        vmcs::write(field::HOST_CS_SELECTOR, (cpu::read_cs() & !7) as u64);
        vmcs::write(field::HOST_SS_SELECTOR, (cpu::read_ss() & !7) as u64);
        vmcs::write(field::HOST_DS_SELECTOR, (cpu::read_ds() & !7) as u64);
        vmcs::write(field::HOST_ES_SELECTOR, (cpu::read_es() & !7) as u64);
        vmcs::write(field::HOST_FS_SELECTOR, (cpu::read_fs() & !7) as u64);
        vmcs::write(field::HOST_GS_SELECTOR, (cpu::read_gs() & !7) as u64);
        vmcs::write(field::HOST_TR_SELECTOR, (tr & !7) as u64);

        vmcs::write(field::HOST_FS_BASE, cpu::rdmsr(cpu::IA32_FS_BASE));
        vmcs::write(field::HOST_GS_BASE, cpu::rdmsr(cpu::IA32_GS_BASE));
        vmcs::write(field::HOST_TR_BASE, gdt::tss_base());
        vmcs::write(field::HOST_GDTR_BASE, gdtr.base);
        vmcs::write(field::HOST_IDTR_BASE, idtr.base);

        vmcs::write(
            field::HOST_IA32_SYSENTER_CS,
            cpu::rdmsr(cpu::IA32_SYSENTER_CS),
        );
        vmcs::write(
            field::HOST_IA32_SYSENTER_ESP,
            cpu::rdmsr(cpu::IA32_SYSENTER_ESP),
        );
        vmcs::write(
            field::HOST_IA32_SYSENTER_EIP,
            cpu::rdmsr(cpu::IA32_SYSENTER_EIP),
        );
        vmcs::write(field::HOST_IA32_EFER, cpu::rdmsr(cpu::IA32_EFER));
        vmcs::write(field::HOST_IA32_PAT, cpu::rdmsr(cpu::IA32_PAT));

        // HOST_RSP は vmx_run が毎回設定する
        vmcs::write(field::HOST_RIP, vmx_exit_entry as *const () as u64);
    }

    fn setup_guest_state(&self, config: &GuestConfig) {
        let fixed = guest_fixed_bits();
        vmcs::write(
            field::GUEST_CR0,
            (config.cr0 | fixed.cr0_fixed1) & fixed.cr0_allowed,
        );
        vmcs::write(field::CR0_READ_SHADOW, config.cr0);
        vmcs::write(field::GUEST_CR3, config.cr3);
        vmcs::write(
            field::GUEST_CR4,
            (config.cr4 | fixed.cr4_fixed1 | CR4_VMXE) & fixed.cr4_allowed,
        );
        vmcs::write(field::CR4_READ_SHADOW, config.cr4 & !CR4_VMXE);
        vmcs::write(field::GUEST_DR7, 0x400);

        for (segment, selector, base, limit, ar) in [
            (
                &config.cs,
                field::GUEST_CS_SELECTOR,
                field::GUEST_CS_BASE,
                field::GUEST_CS_LIMIT,
                field::GUEST_CS_AR_BYTES,
            ),
            (
                &config.ss,
                field::GUEST_SS_SELECTOR,
                field::GUEST_SS_BASE,
                field::GUEST_SS_LIMIT,
                field::GUEST_SS_AR_BYTES,
            ),
            (
                &config.ds,
                field::GUEST_DS_SELECTOR,
                field::GUEST_DS_BASE,
                field::GUEST_DS_LIMIT,
                field::GUEST_DS_AR_BYTES,
            ),
            (
                &config.es,
                field::GUEST_ES_SELECTOR,
                field::GUEST_ES_BASE,
                field::GUEST_ES_LIMIT,
                field::GUEST_ES_AR_BYTES,
            ),
            (
                &config.fs,
                field::GUEST_FS_SELECTOR,
                field::GUEST_FS_BASE,
                field::GUEST_FS_LIMIT,
                field::GUEST_FS_AR_BYTES,
            ),
            (
                &config.gs,
                field::GUEST_GS_SELECTOR,
                field::GUEST_GS_BASE,
                field::GUEST_GS_LIMIT,
                field::GUEST_GS_AR_BYTES,
            ),
            (
                &config.tr,
                field::GUEST_TR_SELECTOR,
                field::GUEST_TR_BASE,
                field::GUEST_TR_LIMIT,
                field::GUEST_TR_AR_BYTES,
            ),
            (
                &config.ldtr,
                field::GUEST_LDTR_SELECTOR,
                field::GUEST_LDTR_BASE,
                field::GUEST_LDTR_LIMIT,
                field::GUEST_LDTR_AR_BYTES,
            ),
        ] {
            vmcs::write(selector, segment.selector as u64);
            vmcs::write(base, segment.base);
            vmcs::write(limit, segment.limit as u64);
            vmcs::write(ar, segment.access_rights as u64);
        }

        vmcs::write(field::GUEST_GDTR_BASE, config.gdtr.base);
        vmcs::write(field::GUEST_GDTR_LIMIT, config.gdtr.limit as u64);
        vmcs::write(field::GUEST_IDTR_BASE, config.idtr.base);
        vmcs::write(field::GUEST_IDTR_LIMIT, config.idtr.limit as u64);

        vmcs::write(field::GUEST_IA32_EFER, config.efer);
        // ゲストが自分で設定するまでは、ファームウェアが設定した PAT をそのまま使わせる
        vmcs::write(field::GUEST_IA32_PAT, cpu::rdmsr(cpu::IA32_PAT));
        vmcs::write(field::GUEST_IA32_DEBUGCTL, 0);
        vmcs::write(field::GUEST_SYSENTER_CS, 0);
        vmcs::write(field::GUEST_SYSENTER_ESP, 0);
        vmcs::write(field::GUEST_SYSENTER_EIP, 0);

        vmcs::write(field::GUEST_RIP, config.rip);
        vmcs::write(field::GUEST_RSP, config.rsp);
        vmcs::write(field::GUEST_RFLAGS, config.rflags);
        vmcs::write(field::GUEST_ACTIVITY_STATE, 0);
        vmcs::write(field::GUEST_INTERRUPTIBILITY_INFO, 0);
        vmcs::write(field::GUEST_PENDING_DBG_EXCEPTIONS, 0);
        vmcs::write(field::VMCS_LINK_POINTER, u64::MAX);
    }

    /// ゲストを実行し、exit を処理し続ける。ゲストを止める必要が生じたら理由を返す
    pub fn run(&mut self) -> StopReason {
        loop {
            let failed = unsafe { vmx_run(&mut self.regs, self.launched as u64) };
            if failed != 0 {
                let error = vmcs::vmread(field::VM_INSTRUCTION_ERROR).ok();
                println!(
                    "[vmx] VM{} failed: VM-instruction error = {:?}",
                    if self.launched { "RESUME" } else { "LAUNCH" },
                    error
                );
                exit::dump_guest_state(self);
                return StopReason::Fatal;
            }
            self.launched = true;

            match exit::handle(self) {
                ExitAction::Resume => sync_ia32e_mode(),
                ExitAction::Stop(reason) => return reason,
            }
        }
    }

    pub fn vmcs_address(&self) -> u64 {
        self.vmcs
    }
}
