//! Intel VT-x の基盤

pub mod ept;
mod exit;
pub use exit::StopReason;
pub mod selftest;
pub mod vcpu;
pub mod vmcs;

use core::alloc::Layout;

use console::println;

use super::cpu::{self, CR4_OSXSAVE, CR4_VMXE};
use vmcs::VmxError;

// フィールドは panic 時の Debug 出力で使う
#[allow(dead_code)]
#[derive(Debug)]
pub enum VmxInitError {
    /// CPUID.1:ECX.VMX が 0
    NotSupported,
    /// IA32_FEATURE_CONTROL がロックされていて、SMX 外での VMXON が許可されていない
    DisabledByFirmware,
    Vmxon(VmxError),
}

const PAGE_SIZE: usize = 4096;

/// VMXON 領域や VMCS 用に、ゼロクリアした 4KiB の物理ページを確保する。
///
/// ヒープは直接マップ（恒等マップ）の範囲にあるので、返り値はそのまま物理アドレスとして使える
/// （docs/memory.md）。
pub(crate) fn alloc_zeroed_page() -> u64 {
    let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        panic!("out of memory while allocating a page");
    }
    ptr as u64
}

/// IA32_VMX_BASIC のビット 30:0。VMXON 領域と VMCS の先頭に書く
pub(crate) fn vmcs_revision_id() -> u32 {
    (cpu::rdmsr(cpu::IA32_VMX_BASIC) & 0x7fff_ffff) as u32
}

/// CR0 / CR4 のうち VMX 動作中に 1 に固定されるビットと、0 に固定されるビット
pub(crate) struct FixedBits {
    pub cr0_fixed1: u64,
    pub cr0_allowed: u64,
    pub cr4_fixed1: u64,
    pub cr4_allowed: u64,
}

pub(crate) fn fixed_bits() -> FixedBits {
    FixedBits {
        cr0_fixed1: cpu::rdmsr(cpu::IA32_VMX_CR0_FIXED0),
        cr0_allowed: cpu::rdmsr(cpu::IA32_VMX_CR0_FIXED1),
        cr4_fixed1: cpu::rdmsr(cpu::IA32_VMX_CR4_FIXED0),
        cr4_allowed: cpu::rdmsr(cpu::IA32_VMX_CR4_FIXED1),
    }
}

/// ゲストに適用する固定ビット。unrestricted guest を使うので CR0.PE / PG は固定しない
pub(crate) fn guest_fixed_bits() -> FixedBits {
    const CR0_PE: u64 = 1 << 0;
    const CR0_PG: u64 = 1 << 31;
    let mut fixed = fixed_bits();
    fixed.cr0_fixed1 &= !(CR0_PE | CR0_PG);
    fixed
}

/// 現在の CPU で VMX root に入る。
///
/// TODO(#8): AP でも呼ぶ。VMXON 領域は CPU ごとに必要
pub fn enable() -> Result<(), VmxInitError> {
    const FEATURE_CONTROL_LOCK: u64 = 1 << 0;
    const FEATURE_CONTROL_VMXON_OUTSIDE_SMX: u64 = 1 << 2;

    if cpu::cpuid(1, 0).ecx & (1 << 5) == 0 {
        return Err(VmxInitError::NotSupported);
    }

    let feature_control = cpu::rdmsr(cpu::IA32_FEATURE_CONTROL);
    if feature_control & FEATURE_CONTROL_LOCK == 0 {
        unsafe {
            cpu::wrmsr(
                cpu::IA32_FEATURE_CONTROL,
                feature_control | FEATURE_CONTROL_LOCK | FEATURE_CONTROL_VMXON_OUTSIDE_SMX,
            );
        }
    } else if feature_control & FEATURE_CONTROL_VMXON_OUTSIDE_SMX == 0 {
        return Err(VmxInitError::DisabledByFirmware);
    }

    // ゲストの XSETBV をホストで代行するため、ホストでも CR4.OSXSAVE を立てておく
    // （HOST_CR4 は VMCS の初期化時にこの値を読む）
    let osxsave = if cpu::cpuid(1, 0).ecx & (1 << 26) != 0 {
        CR4_OSXSAVE
    } else {
        0
    };
    let fixed = fixed_bits();
    unsafe {
        cpu::write_cr0((cpu::read_cr0() | fixed.cr0_fixed1) & fixed.cr0_allowed);
        cpu::write_cr4(
            (cpu::read_cr4() | fixed.cr4_fixed1 | CR4_VMXE | osxsave) & fixed.cr4_allowed,
        );
    }

    let vmxon_region = alloc_zeroed_page();
    unsafe {
        *(vmxon_region as *mut u32) = vmcs_revision_id();
        vmcs::vmxon(vmxon_region).map_err(VmxInitError::Vmxon)?;
    }

    println!("[vmx] VMXON done (revision id {:#x})", vmcs_revision_id());
    Ok(())
}
