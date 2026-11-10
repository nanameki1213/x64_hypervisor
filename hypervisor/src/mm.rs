//! 物理メモリの分割。
//!
//! HV が持つ物理メモリは HV のイメージと固定サイズのヒープだけで、残りはすべて管理 VM に渡す。
//! 詳細は docs/memory.md を参照。

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
#[repr(C)]
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
///
/// # Safety
/// `map_buffer` は `map_size` バイトの UEFI メモリマップを指していること
pub unsafe fn init(map_buffer: *const u8, map_size: usize, descriptor_size: usize) -> MemoryLayout {
    let image = image_range();
    let mut heap: Option<Range<u64>> = None;
    let mut memory_map_top = 0;

    for i in 0..map_size / descriptor_size {
        let desc = unsafe { &*(map_buffer.add(i * descriptor_size) as *const EfiMemoryDescriptor) };
        let start = desc.physical_start;
        let end = start + desc.number_of_pages * PAGE_SIZE_4K;
        memory_map_top = memory_map_top.max(end);

        if desc.memory_type != EfiMemoryType::ConventionalMemory as u32 {
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
