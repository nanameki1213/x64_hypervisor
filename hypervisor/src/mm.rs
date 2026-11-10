//! 物理メモリの分割。
//!
//! HV が持つ物理メモリは HV のイメージと固定サイズのヒープだけで、残りはすべて管理 VM に渡す。
//! 詳細は docs/memory.md を参照。

use alloc::vec::Vec;
use core::ops::Range;

/// HV のヒープのサイズ。VMCS や EPT のページテーブルなど、HV の動的な確保はすべてここから行う
pub const HV_HEAP_SIZE: u64 = 32 * 1024 * 1024;

pub const PAGE_SIZE_4K: u64 = 0x1000;
pub const PAGE_SIZE_2M: u64 = 0x20_0000;
pub const PAGE_SIZE_1G: u64 = 0x4000_0000;

/// boot.S が恒等マップしている範囲（HV はこの範囲の物理メモリにしかアクセスできない）
/// TODO: 512GiB を超える物理メモリを持つ機材に対応するときに直接マップを広げる
pub const DIRECT_MAP_END: u64 = 512 * PAGE_SIZE_1G;

/// EFI_MEMORY_DESCRIPTOR
#[repr(C)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

#[allow(dead_code)]
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EfiMemoryType {
    ReservedMemoryType,
    LoaderCode,
    LoaderData,
    BootServicesCode,
    BootServicesData,
    RuntimeServicesCode,
    RuntimeServicesData,
    ConventionalMemory,
    UnusableMemory,
    AcpiReclaimMemory,
    AcpiMemoryNvs,
    MemoryMappedIo,
    MemoryMappedIoPortSpace,
    PalCode,
    PersistentMemory,
    MaxMemoryType,
}

impl EfiMemoryDescriptor {
    pub fn range(&self) -> Range<u64> {
        self.physical_start..self.physical_start + self.number_of_pages * PAGE_SIZE_4K
    }

    pub fn is_type(&self, t: EfiMemoryType) -> bool {
        self.memory_type == t as u32
    }

    /// `ExitBootServices` の後に自由に使えるメモリか
    pub fn is_free_after_exit_boot_services(&self) -> bool {
        self.is_type(EfiMemoryType::ConventionalMemory)
            || self.is_type(EfiMemoryType::BootServicesCode)
            || self.is_type(EfiMemoryType::BootServicesData)
    }
}

/// ブートローダから受け取った UEFI のメモリマップ（LoaderData にある）
#[derive(Clone, Copy)]
pub struct EfiMemoryMap {
    buffer: *const u8,
    size: usize,
    descriptor_size: usize,
}

impl EfiMemoryMap {
    /// # Safety
    /// `buffer` は `size` バイトの UEFI メモリマップを指し、以後上書きされないこと
    pub unsafe fn new(buffer: *const u8, size: usize, descriptor_size: usize) -> Self {
        EfiMemoryMap {
            buffer,
            size,
            descriptor_size,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static EfiMemoryDescriptor> + '_ {
        (0..self.size / self.descriptor_size).map(|i| unsafe {
            &*(self.buffer.add(i * self.descriptor_size) as *const EfiMemoryDescriptor)
        })
    }
}

/// 重ならない物理アドレスの範囲の集合（昇順）
#[derive(Clone, Debug, Default)]
pub struct RangeSet {
    ranges: Vec<Range<u64>>,
}

impl RangeSet {
    pub fn add(&mut self, range: Range<u64>) {
        if range.is_empty() {
            return;
        }
        let mut merged = range;
        self.ranges.retain(|r| {
            // 重なるか接する範囲は 1 つにまとめる
            if r.start <= merged.end && merged.start <= r.end {
                merged = merged.start.min(r.start)..merged.end.max(r.end);
                false
            } else {
                true
            }
        });
        let pos = self.ranges.partition_point(|r| r.start < merged.start);
        self.ranges.insert(pos, merged);
    }

    pub fn remove(&mut self, range: &Range<u64>) {
        let mut result = Vec::with_capacity(self.ranges.len() + 1);
        for r in self.ranges.drain(..) {
            if r.end <= range.start || range.end <= r.start {
                result.push(r);
                continue;
            }
            if r.start < range.start {
                result.push(r.start..range.start);
            }
            if range.end < r.end {
                result.push(range.end..r.end);
            }
        }
        self.ranges = result;
    }

    pub fn contains_range(&self, range: &Range<u64>) -> bool {
        self.ranges
            .iter()
            .any(|r| r.start <= range.start && range.end <= r.end)
    }

    /// `[min, max)` に収まる `align` アラインの `size` バイトを、もっとも低いアドレスから取り出す
    pub fn take(&mut self, size: u64, align: u64, min: u64, max: u64) -> Option<Range<u64>> {
        let found = self.ranges.iter().find_map(|r| {
            let start = align_up(r.start.max(min), align);
            let end = start.checked_add(size)?;
            (end <= r.end && end <= max).then_some(start..end)
        })?;
        self.remove(&found);
        Some(found)
    }
}

/// `ExitBootServices` の後に空いているメモリから、HV のメモリを除いたもの。
/// ブートローダが LoaderData に置いたファイル（bzImage など）は含まれない
pub fn free_memory(memory_map: &EfiMemoryMap, layout: &MemoryLayout) -> RangeSet {
    let mut free = RangeSet::default();
    for d in memory_map
        .iter()
        .filter(|d| d.is_free_after_exit_boot_services())
    {
        free.add(d.range());
    }
    for r in layout.reserved() {
        free.remove(&r);
    }
    free
}

/// HV が使う物理メモリの配置
#[derive(Clone, Debug)]
pub struct MemoryLayout {
    /// HV のイメージ（2MiB 単位に切り上げ）
    pub image: Range<u64>,
    /// HV のヒープ
    pub heap: Range<u64>,
    /// UEFI のメモリマップに載っている範囲の上端
    pub memory_map_top: u64,
}

impl MemoryLayout {
    /// 管理 VM からアクセスさせない物理アドレスの範囲
    pub fn reserved(&self) -> [Range<u64>; 2] {
        [self.image.clone(), self.heap.clone()]
    }
}

unsafe extern "C" {
    /// boot.S が保存した、HV のイメージのロード先物理アドレス
    static hv_image_phys_base: u64;
    /// hypervisor.ld が定義する、HV のイメージの仮想アドレス範囲
    static __hv_image_start: u8;
    static __hv_image_end: u8;
}

pub const fn align_down(x: u64, align: u64) -> u64 {
    x & !(align - 1)
}

pub const fn align_up(x: u64, align: u64) -> u64 {
    align_down(x + align - 1, align)
}

fn image_range() -> Range<u64> {
    let start = unsafe { hv_image_phys_base };
    let size = (&raw const __hv_image_end) as u64 - (&raw const __hv_image_start) as u64;
    // boot.S はロード先が 2MiB アラインであることを確認している
    start..align_up(start + size, PAGE_SIZE_2M)
}

/// UEFI のメモリマップから、HV のヒープに使う領域を決める。
///
/// 管理 VM の低位メモリ（リアルモードのトランポリンなど）と競合しないよう、
/// ConventionalMemory のうちもっとも高位で、直接マップの範囲に収まる 2MiB アラインの領域を選ぶ。
/// 2MiB アラインにしておくと、管理 VM の EPT で周囲を大きいページのままマップできる。
pub fn init(memory_map: &EfiMemoryMap) -> MemoryLayout {
    let image = image_range();
    let mut heap: Option<Range<u64>> = None;
    let mut memory_map_top = 0;

    for desc in memory_map.iter() {
        let Range { start, end } = desc.range();
        memory_map_top = memory_map_top.max(end);

        if !desc.is_type(EfiMemoryType::ConventionalMemory) {
            continue;
        }
        let candidate_end = align_down(end.min(DIRECT_MAP_END), PAGE_SIZE_2M);
        let Some(candidate_start) = candidate_end.checked_sub(HV_HEAP_SIZE) else {
            continue;
        };
        if candidate_start < align_up(start, PAGE_SIZE_2M) {
            continue;
        }
        if heap.as_ref().is_none_or(|h| candidate_start > h.start) {
            heap = Some(candidate_start..candidate_end);
        }
    }

    // HV のイメージは 2MiB 単位で管理 VM から隠すので、切り上げた部分まで
    // ブートローダが確保した（LoaderCode）領域でなければならない。
    // まだヒープがないので、メモリ確保をせずに先頭から順にたどる
    let mut covered = image.start;
    while covered < image.end {
        let Some(next) = memory_map
            .iter()
            .filter(|d| d.is_type(EfiMemoryType::LoaderCode))
            .map(|d| d.range())
            .find(|r| r.contains(&covered))
        else {
            panic!(
                "HV image {:#x}-{:#x} (rounded up to 2MiB) is not entirely allocated as LoaderCode (gap at {:#x})",
                image.start, image.end, covered
            );
        };
        covered = next.end;
    }

    let heap = heap.unwrap_or_else(|| {
        panic!(
            "no ConventionalMemory region can hold the {} MiB HV heap",
            HV_HEAP_SIZE >> 20
        )
    });
    MemoryLayout {
        image,
        heap,
        memory_map_top,
    }
}
