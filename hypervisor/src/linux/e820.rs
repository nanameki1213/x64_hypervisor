//! UEFI のメモリマップから、管理 VM に渡す e820 テーブルを作る

use alloc::vec::Vec;
use core::ops::Range;

use crate::mm::{EfiMemoryMap, EfiMemoryType};

pub const RAM: u32 = 1;
pub const RESERVED: u32 = 2;
pub const ACPI: u32 = 3;
pub const NVS: u32 = 4;
pub const UNUSABLE: u32 = 5;
pub const PMEM: u32 = 7;

/// boot_params（zero page）に入る e820 エントリの最大数
pub const MAX_ENTRIES_ZEROPAGE: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub range: Range<u64>,
    pub kind: u32,
}

/// UEFI のメモリタイプに対応する e820 のタイプ（Linux の EFI stub の setup_e820 と同じ対応）
fn kind_of(memory_type: u32) -> u32 {
    use EfiMemoryType::*;
    let t = |e: EfiMemoryType| e as u32;
    match memory_type {
        m if m == t(LoaderCode)
            || m == t(LoaderData)
            || m == t(BootServicesCode)
            || m == t(BootServicesData)
            || m == t(ConventionalMemory) =>
        {
            RAM
        }
        m if m == t(AcpiReclaimMemory) => ACPI,
        m if m == t(AcpiMemoryNvs) => NVS,
        m if m == t(UnusableMemory) => UNUSABLE,
        m if m == t(PersistentMemory) => PMEM,
        // RuntimeServicesCode / Data、MMIO、予約領域など。
        // 管理 VM には EFI runtime services を使わせない（efi_info を渡さない）ので、
        // runtime services の領域も予約として扱う
        _ => RESERVED,
    }
}

/// e820 テーブルを作る。`hidden` の範囲（HV のメモリ）は予約にする。
///
/// 除外（穴にする）ではなく予約にするのは、Linux が穴を PCI の MMIO の割り当て先に使うことがあるため
pub fn build(memory_map: &EfiMemoryMap, hidden: &[Range<u64>]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = memory_map
        .iter()
        .filter(|d| d.number_of_pages != 0)
        .map(|d| Entry {
            range: d.range(),
            kind: kind_of(d.memory_type),
        })
        .collect();
    entries.sort_unstable_by_key(|e| e.range.start);

    for h in hidden {
        entries = overlay(entries, h, RESERVED);
    }
    merge(entries)
}

/// `range` と重なる部分のタイプを `kind` にする
fn overlay(entries: Vec<Entry>, range: &Range<u64>, kind: u32) -> Vec<Entry> {
    let mut result = Vec::with_capacity(entries.len() + 2);
    for e in entries {
        let start = e.range.start.max(range.start);
        let end = e.range.end.min(range.end);
        if start >= end {
            result.push(e);
            continue;
        }
        if e.range.start < start {
            result.push(Entry {
                range: e.range.start..start,
                kind: e.kind,
            });
        }
        result.push(Entry {
            range: start..end,
            kind,
        });
        if end < e.range.end {
            result.push(Entry {
                range: end..e.range.end,
                kind: e.kind,
            });
        }
    }
    result
}

/// 隣接していてタイプが同じエントリをまとめる
fn merge(entries: Vec<Entry>) -> Vec<Entry> {
    let mut result: Vec<Entry> = Vec::with_capacity(entries.len());
    for e in entries {
        if let Some(last) = result.last_mut()
            && last.kind == e.kind
            && last.range.end == e.range.start
        {
            last.range.end = e.range.end;
            continue;
        }
        result.push(e);
    }
    result
}

pub fn kind_name(kind: u32) -> &'static str {
    match kind {
        RAM => "usable",
        RESERVED => "reserved",
        ACPI => "ACPI data",
        NVS => "ACPI NVS",
        UNUSABLE => "unusable",
        PMEM => "persistent",
        _ => "?",
    }
}
