//! EPT（Extended Page Table）
//!
//! 管理 VM はゲスト物理アドレス = ホスト物理アドレスの恒等マップで動かし、
//! HV が使う物理メモリ（holes）だけをマップしない。

use alloc::vec::Vec;
use core::arch::asm;
use core::ops::Range;

use super::alloc_zeroed_page;
use crate::arch::x86_64::cpu;
use crate::arch::x86_64::mtrr::{self, Mtrr};
use crate::mm::{MemoryLayout, PAGE_SIZE_1G, PAGE_SIZE_4K, align_down, align_up};

const IA32_VMX_EPT_VPID_CAP: u32 = 0x48c;
const CAP_WALK_LENGTH_4: u64 = 1 << 6;
const CAP_MEMTYPE_WB: u64 = 1 << 14;
const CAP_PAGE_2M: u64 = 1 << 16;
const CAP_PAGE_1G: u64 = 1 << 17;
const CAP_INVEPT: u64 = 1 << 20;
const CAP_INVEPT_SINGLE: u64 = 1 << 25;
const CAP_INVEPT_ALL: u64 = 1 << 26;

const READ: u64 = 1 << 0;
const WRITE: u64 = 1 << 1;
const EXECUTE: u64 = 1 << 2;
const RWX: u64 = READ | WRITE | EXECUTE;
const MEMTYPE_SHIFT: u64 = 3;
const LARGE_PAGE: u64 = 1 << 7;
const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

const ENTRIES: usize = 512;

#[derive(Debug)]
pub enum EptError {
    /// CPU が 4 段の EPT または WB の EPT 構造をサポートしていない
    NotSupported,
}

/// EPT violation の原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationError {
    /// HV が使う物理メモリへのアクセス
    HvMemory,
    /// マップ済みのページへのアクセス（権限違反）。今は権限を絞ったマップを作らないので起きないはず
    AlreadyMapped,
}

/// マップの統計（デバッグ用）
#[derive(Debug, Default, Clone, Copy)]
pub struct EptStats {
    pub pages_1g: usize,
    pub pages_2m: usize,
    pub pages_4k: usize,
    pub tables: usize,
}

pub struct Ept {
    pml4: u64,
    holes: Vec<Range<u64>>,
    mtrr: Mtrr,
    page_1g: bool,
    page_2m: bool,
}

/// レベル（4 = PML4, 3 = PDPT, 2 = PD, 1 = PT）の 1 エントリが表すサイズ
const fn entry_size(level: u32) -> u64 {
    PAGE_SIZE_4K << (9 * (level - 1))
}

fn table(phys: u64) -> &'static mut [u64; ENTRIES] {
    // ページテーブルは HV のヒープ（恒等マップ）にある
    unsafe { &mut *(phys as *mut [u64; ENTRIES]) }
}

impl Ept {
    pub fn new(holes: &[Range<u64>]) -> Result<Self, EptError> {
        let cap = cpu::rdmsr(IA32_VMX_EPT_VPID_CAP);
        if cap & CAP_WALK_LENGTH_4 == 0 || cap & CAP_MEMTYPE_WB == 0 {
            return Err(EptError::NotSupported);
        }
        Ok(Ept {
            pml4: alloc_zeroed_page(),
            holes: holes.to_vec(),
            mtrr: Mtrr::read(),
            page_1g: cap & CAP_PAGE_1G != 0,
            page_2m: cap & CAP_PAGE_2M != 0,
        })
    }

    /// VMCS の EPT_POINTER に設定する値（WB、4 段、A/D フラグなし）
    pub fn eptp(&self) -> u64 {
        self.pml4 | mtrr::WB as u64 | (3 << 3)
    }

    fn overlaps_hole(&self, range: &Range<u64>) -> bool {
        self.holes
            .iter()
            .any(|h| h.start < range.end && range.start < h.end)
    }

    fn inside_hole(&self, range: &Range<u64>) -> bool {
        self.holes
            .iter()
            .any(|h| h.start <= range.start && range.end <= h.end)
    }

    fn large_page_allowed(&self, level: u32) -> bool {
        match level {
            3 => self.page_1g,
            2 => self.page_2m,
            _ => false,
        }
    }

    /// `range` を恒等マップする。マップ済みのページは変更せず、holes にかかるページはマップしない。
    /// ページサイズは、holes にかからずメモリタイプが一様な範囲で最大のものを選ぶ
    pub fn map_identity(&mut self, range: Range<u64>) {
        let range = align_down(range.start, PAGE_SIZE_4K)..align_up(range.end, PAGE_SIZE_4K);
        self.fill_identity(self.pml4, 4, range);
    }

    fn fill_identity(&self, table_phys: u64, level: u32, range: Range<u64>) {
        let size = entry_size(level);
        let entries = table(table_phys);
        let mut addr = range.start;
        while addr < range.end {
            let entry_start = align_down(addr, size);
            let entry_end = entry_start + size;
            let sub = addr..range.end.min(entry_end);
            let index = ((entry_start / size) % ENTRIES as u64) as usize;
            addr = sub.end;

            if self.inside_hole(&sub) {
                continue;
            }
            let entry = entries[index];
            let present = entry & RWX != 0;
            let is_leaf = level == 1 || entry & LARGE_PAGE != 0;
            if present && is_leaf {
                continue;
            }
            if present {
                self.fill_identity(entry & ADDR_MASK, level - 1, sub);
                continue;
            }

            let whole = sub.start == entry_start && sub.end == entry_end;
            if whole && (level == 1 || self.large_page_allowed(level)) {
                let page = entry_start..entry_end;
                if !self.overlaps_hole(&page) {
                    // 4KiB では MTRR のタイプは必ず一様になる
                    if let Some(mem_type) = self.mtrr.memory_type(entry_start, size) {
                        let large = if level == 1 { 0 } else { LARGE_PAGE };
                        entries[index] =
                            entry_start | RWX | ((mem_type as u64) << MEMTYPE_SHIFT) | large;
                        continue;
                    }
                }
            }
            if level == 1 {
                // 4KiB ページが hole にかかる（hole は 4KiB アラインなので、hole の中にある）
                continue;
            }
            let child = alloc_zeroed_page();
            entries[index] = child | RWX;
            self.fill_identity(child, level - 1, sub);
        }
    }

    /// ゲスト物理アドレスを変換する。マップされていなければ None
    pub fn translate(&self, gpa: u64) -> Option<u64> {
        let mut table_phys = self.pml4;
        for level in (1..=4).rev() {
            let size = entry_size(level);
            let index = ((gpa / size) % ENTRIES as u64) as usize;
            let entry = table(table_phys)[index];
            if entry & RWX == 0 {
                return None;
            }
            if level == 1 || entry & LARGE_PAGE != 0 {
                let base = entry & ADDR_MASK & !(size - 1);
                return Some(base + (gpa & (size - 1)));
            }
            table_phys = entry & ADDR_MASK;
        }
        unreachable!()
    }

    /// EPT violation の処理。HV のメモリ以外なら、そのアドレスを含む 1GiB を恒等マップする
    /// （64bit BAR など、起動時にマップしなかった高位の MMIO へのアクセスで起きる）
    pub fn handle_violation(&mut self, gpa: u64) -> Result<(), ViolationError> {
        if self.holes.iter().any(|h| h.contains(&gpa)) {
            return Err(ViolationError::HvMemory);
        }
        if self.translate(gpa).is_some() {
            return Err(ViolationError::AlreadyMapped);
        }
        let start = align_down(gpa, PAGE_SIZE_1G);
        self.map_identity(start..start + PAGE_SIZE_1G);
        // 存在しないエントリは TLB にキャッシュされないが、念のため無効化しておく
        invept(self.eptp());
        Ok(())
    }

    pub fn stats(&self) -> EptStats {
        let mut stats = EptStats::default();
        Self::count(self.pml4, 4, &mut stats);
        stats
    }

    fn count(table_phys: u64, level: u32, stats: &mut EptStats) {
        stats.tables += 1;
        for &entry in table(table_phys).iter() {
            if entry & RWX == 0 {
                continue;
            }
            match level {
                1 => stats.pages_4k += 1,
                2 if entry & LARGE_PAGE != 0 => stats.pages_2m += 1,
                3 if entry & LARGE_PAGE != 0 => stats.pages_1g += 1,
                _ => Self::count(entry & ADDR_MASK, level - 1, stats),
            }
        }
    }
}

/// 起動時に管理 VM の EPT で恒等マップする範囲。
/// 下位 4GiB（MMIO を含む）と UEFI のメモリマップに載っている範囲をまとめてマップし、
/// それより上（64bit BAR など）は EPT violation のときに遅延マップする
pub fn initial_identity_range(layout: &MemoryLayout) -> Range<u64> {
    0..align_up(layout.memory_map_top.max(4 * PAGE_SIZE_1G), PAGE_SIZE_1G)
}

/// 管理 VM 用の EPT。HV のメモリ以外を恒等マップする
pub fn management_vm(layout: &MemoryLayout) -> Result<Ept, EptError> {
    let mut ept = Ept::new(&layout.reserved())?;
    ept.map_identity(initial_identity_range(layout));
    Ok(ept)
}

/// `eptp` に関する TLB とページング構造のキャッシュを無効化する
pub fn invept(eptp: u64) {
    const SINGLE_CONTEXT: u64 = 1;
    const ALL_CONTEXT: u64 = 2;

    let cap = cpu::rdmsr(IA32_VMX_EPT_VPID_CAP);
    if cap & CAP_INVEPT == 0 {
        return;
    }
    let kind = if cap & CAP_INVEPT_SINGLE != 0 {
        SINGLE_CONTEXT
    } else if cap & CAP_INVEPT_ALL != 0 {
        ALL_CONTEXT
    } else {
        return;
    };
    let descriptor: [u64; 2] = [eptp, 0];
    unsafe {
        asm!("invept {}, [{}]", in(reg) kind, in(reg) &descriptor, options(nostack));
    }
}
