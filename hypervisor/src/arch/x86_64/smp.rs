//! マルチプロセッサ（#8）。vCPU と物理 CPU は 1:1 で固定する

use acpi::{Madt, Xsdt};
use alloc::vec::Vec;

use super::cpu;

/// 起動時に MADT から調べた CPU の構成
#[derive(Debug)]
pub struct CpuTopology {
    /// 使える CPU の APIC ID（MADT の順。BSP を含む）
    pub apic_ids: Vec<u32>,
    /// BSP（この関数を実行している CPU）の APIC ID
    pub bsp_apic_id: u32,
}

/// 現在の CPU の APIC ID。x2APIC の ID（CPUID 0xB）が取れればそちらを使う
pub fn current_apic_id() -> u32 {
    if cpu::cpuid(0, 0).eax >= 0xb {
        let r = cpu::cpuid(0xb, 0);
        // サブリーフ 0 の EBX が 0 なら leaf 0xB は無効
        if r.ebx != 0 {
            return r.edx;
        }
    }
    cpu::cpuid(1, 0).ebx >> 24
}

/// # Safety
/// `xsdt` のエントリが、読めるメモリ上の ACPI テーブルを指していること
pub unsafe fn discover(xsdt: &Xsdt) -> CpuTopology {
    let madt = unsafe { xsdt.find::<Madt>() }.expect("MADT is not found in the XSDT");
    let topology = CpuTopology {
        apic_ids: madt.apic_ids().collect(),
        bsp_apic_id: current_apic_id(),
    };
    assert!(
        topology.apic_ids.contains(&topology.bsp_apic_id),
        "the BSP (APIC ID {}) is not listed in the MADT",
        topology.bsp_apic_id
    );
    topology
}
