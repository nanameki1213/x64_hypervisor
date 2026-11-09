//! 特権命令の薄いラッパ

use core::arch::asm;

pub const IA32_FEATURE_CONTROL: u32 = 0x3a;
pub const IA32_SYSENTER_CS: u32 = 0x174;
pub const IA32_SYSENTER_ESP: u32 = 0x175;
pub const IA32_SYSENTER_EIP: u32 = 0x176;
pub const IA32_PAT: u32 = 0x277;
pub const IA32_VMX_BASIC: u32 = 0x480;
pub const IA32_VMX_PINBASED_CTLS: u32 = 0x481;
pub const IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
pub const IA32_VMX_EXIT_CTLS: u32 = 0x483;
pub const IA32_VMX_ENTRY_CTLS: u32 = 0x484;
pub const IA32_VMX_CR0_FIXED0: u32 = 0x486;
pub const IA32_VMX_CR0_FIXED1: u32 = 0x487;
pub const IA32_VMX_CR4_FIXED0: u32 = 0x488;
pub const IA32_VMX_CR4_FIXED1: u32 = 0x489;
pub const IA32_VMX_PROCBASED_CTLS2: u32 = 0x48b;
pub const IA32_VMX_TRUE_PINBASED_CTLS: u32 = 0x48d;
pub const IA32_VMX_TRUE_PROCBASED_CTLS: u32 = 0x48e;
pub const IA32_VMX_TRUE_EXIT_CTLS: u32 = 0x48f;
pub const IA32_VMX_TRUE_ENTRY_CTLS: u32 = 0x490;
/// VMX 関連 MSR の範囲（ゲストには見せない）
pub const IA32_VMX_MSR_FIRST: u32 = 0x480;
pub const IA32_VMX_MSR_LAST: u32 = 0x493;
pub const IA32_EFER: u32 = 0xc000_0080;
pub const IA32_FS_BASE: u32 = 0xc000_0100;
pub const IA32_GS_BASE: u32 = 0xc000_0101;

pub const CR4_VMXE: u64 = 1 << 13;
pub const CR4_OSXSAVE: u64 = 1 << 18;

#[derive(Clone, Copy, Debug)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    let r = core::arch::x86_64::__cpuid_count(leaf, subleaf);
    CpuidResult {
        eax: r.eax,
        ebx: r.ebx,
        ecx: r.ecx,
        edx: r.edx,
    }
}

pub fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | lo as u64
}

/// # Safety
/// 存在しない MSR や不正な値を書くと #GP になる
pub unsafe fn wrmsr(msr: u32, value: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nostack, preserves_flags));
    }
}

macro_rules! cr_accessors {
    ($read:ident, $write:ident, $cr:literal) => {
        pub fn $read() -> u64 {
            let v: u64;
            unsafe {
                asm!(concat!("mov {}, ", $cr), out(reg) v, options(nomem, nostack, preserves_flags));
            }
            v
        }

        /// # Safety
        /// 制御レジスタを書き換えるため、呼び出し側で値の妥当性を保証すること
        #[allow(dead_code)]
        pub unsafe fn $write(v: u64) {
            unsafe {
                asm!(concat!("mov ", $cr, ", {}"), in(reg) v, options(nostack, preserves_flags));
            }
        }
    };
}

cr_accessors!(read_cr0, write_cr0, "cr0");
cr_accessors!(read_cr3, write_cr3, "cr3");
cr_accessors!(read_cr4, write_cr4, "cr4");

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DescriptorTablePointer {
    pub limit: u16,
    pub base: u64,
}

pub fn sgdt() -> DescriptorTablePointer {
    let mut p = DescriptorTablePointer::default();
    unsafe {
        asm!("sgdt [{}]", in(reg) &mut p, options(nostack, preserves_flags));
    }
    p
}

pub fn sidt() -> DescriptorTablePointer {
    let mut p = DescriptorTablePointer::default();
    unsafe {
        asm!("sidt [{}]", in(reg) &mut p, options(nostack, preserves_flags));
    }
    p
}

macro_rules! selector_reader {
    ($name:ident, $seg:literal) => {
        pub fn $name() -> u16 {
            let v: u16;
            unsafe {
                asm!(concat!("mov {:x}, ", $seg), out(reg) v, options(nomem, nostack, preserves_flags));
            }
            v
        }
    };
}

selector_reader!(read_cs, "cs");
selector_reader!(read_ss, "ss");
selector_reader!(read_ds, "ds");
selector_reader!(read_es, "es");
selector_reader!(read_fs, "fs");
selector_reader!(read_gs, "gs");

pub fn read_tr() -> u16 {
    let v: u16;
    unsafe {
        asm!("str {:x}", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

pub fn wbinvd() {
    unsafe { asm!("wbinvd", options(nostack, preserves_flags)) };
}

/// # Safety
/// XCR0 に CPU が対応しない値を書くと #GP になる
pub unsafe fn xsetbv(xcr: u32, value: u64) {
    unsafe {
        asm!("xsetbv", in("ecx") xcr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nostack, preserves_flags));
    }
}
