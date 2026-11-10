//! 管理 VM（Linux）の起動。
//!
//! Linux x86 boot protocol（Documentation/arch/x86/boot.rst）の 64bit エントリを使う。
//! ブートローダが ESP から読み込んだ bzImage / initrd / cmdline を受け取り、
//! カーネルの配置、boot_params（zero page）、ゲスト用の GDT とページテーブルを用意して、
//! `code32_start + 0x200` から 64bit モードで実行を始める。詳細は docs/linux-boot.md を参照。

mod e820;

use core::ops::Range;

use boot_info::LinuxImage;
use console::println;

use crate::arch::x86_64::smp;
use crate::arch::x86_64::vmx::StopReason;
use crate::arch::x86_64::vmx::ept;
use crate::arch::x86_64::vmx::vcpu::{DescriptorTable, GuestConfig, Vcpu};
use crate::mm::{
    self, EfiMemoryMap, MemoryLayout, PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, RangeSet,
};

/// setup header（bzImage と boot_params で同じオフセット）
mod hdr {
    pub const SETUP_SECTS: usize = 0x1f1;
    pub const BOOT_FLAG: usize = 0x1fe;
    pub const JUMP_END: usize = 0x201;
    pub const HEADER: usize = 0x202;
    pub const VERSION: usize = 0x206;
    pub const TYPE_OF_LOADER: usize = 0x210;
    pub const LOADFLAGS: usize = 0x211;
    pub const CODE32_START: usize = 0x214;
    pub const RAMDISK_IMAGE: usize = 0x218;
    pub const RAMDISK_SIZE: usize = 0x21c;
    pub const CMD_LINE_PTR: usize = 0x228;
    pub const INITRD_ADDR_MAX: usize = 0x22c;
    pub const KERNEL_ALIGNMENT: usize = 0x230;
    pub const RELOCATABLE_KERNEL: usize = 0x234;
    pub const XLOADFLAGS: usize = 0x236;
    pub const CMDLINE_SIZE: usize = 0x238;
    pub const PREF_ADDRESS: usize = 0x258;
    pub const INIT_SIZE: usize = 0x260;

    pub const BOOT_FLAG_MAGIC: u16 = 0xaa55;
    pub const HEADER_MAGIC: u32 = u32::from_le_bytes(*b"HdrS");
    /// 64bit エントリ（XLF_KERNEL_64）はプロトコル 2.12 から
    pub const MIN_VERSION: u16 = 0x020c;
    pub const LOADED_HIGH: u8 = 1 << 0;
    pub const XLF_KERNEL_64: u16 = 1 << 0;
    /// type_of_loader: 未登録のブートローダ
    pub const LOADER_UNDEFINED: u8 = 0xff;
}

/// boot_params のうち setup header 以外のフィールド
mod params {
    pub const ACPI_RSDP_ADDR: usize = 0x070;
    pub const EXT_RAMDISK_IMAGE: usize = 0x0c0;
    pub const EXT_RAMDISK_SIZE: usize = 0x0c4;
    pub const EXT_CMD_LINE_PTR: usize = 0x0c8;
    pub const E820_ENTRIES: usize = 0x1e8;
    pub const E820_TABLE: usize = 0x2d0;
    pub const E820_ENTRY_SIZE: usize = 20;
}

/// 64bit エントリはカーネルのロード先からこのオフセット
const ENTRY_64_OFFSET: u64 = 0x200;

/// ゲスト用の起動データ（boot_params、cmdline、GDT、ページテーブル、スタック）の配置
mod boot_data {
    pub const BOOT_PARAMS: u64 = 0x0000;
    pub const CMDLINE: u64 = 0x1000;
    pub const CMDLINE_MAX: usize = 0x1000;
    pub const GDT: u64 = 0x2000;
    pub const PML4: u64 = 0x3000;
    pub const PDPT: u64 = 0x4000;
    /// 下位 4GiB を 2MiB ページで恒等マップするための PD 4 枚
    pub const PD: u64 = 0x5000;
    pub const PD_COUNT: u64 = 4;
    pub const STACK_TOP: u64 = 0xd000;
    pub const SIZE: u64 = 0x1_0000;
}

/// boot protocol が要求するセレクタ（__BOOT_CS / __BOOT_DS）
const BOOT_CS: u16 = 0x10;
const BOOT_DS: u16 = 0x18;
const GDT_ENTRIES: [u64; 4] = [0, 0, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff];

/// カーネルと起動データは、ゲストのページテーブルで恒等マップする下位 4GiB に置く
const PLACEMENT_MAX: u64 = 4 * PAGE_SIZE_1G;
/// 低位 1MiB は Linux 自身が予約する（リアルモードのトランポリンなど）ので、置き場所に使わない
const PLACEMENT_MIN: u64 = 0x10_0000;

/// フィールドは panic のメッセージ（Debug）でだけ使う
#[allow(dead_code)]
#[derive(Debug)]
pub enum LinuxBootError {
    NotBzImage,
    UnsupportedProtocol(u16),
    No64BitEntry,
    CmdlineTooLong { len: usize, max: usize },
    InitrdAboveLimit { end: u64, max: u64 },
    NoSpaceForKernel { size: u64 },
    NoSpaceForBootData,
    TooManyE820Entries(usize),
}

struct SetupHeader<'a> {
    image: &'a [u8],
}

impl<'a> SetupHeader<'a> {
    fn parse(image: &'a [u8]) -> Result<Self, LinuxBootError> {
        if image.len() < 0x1000 {
            return Err(LinuxBootError::NotBzImage);
        }
        let h = SetupHeader { image };
        if h.u16(hdr::BOOT_FLAG) != hdr::BOOT_FLAG_MAGIC || h.u32(hdr::HEADER) != hdr::HEADER_MAGIC
        {
            return Err(LinuxBootError::NotBzImage);
        }
        if h.version() < hdr::MIN_VERSION {
            return Err(LinuxBootError::UnsupportedProtocol(h.version()));
        }
        if h.u16(hdr::XLOADFLAGS) & hdr::XLF_KERNEL_64 == 0 {
            return Err(LinuxBootError::No64BitEntry);
        }
        if h.payload_offset() >= image.len() {
            return Err(LinuxBootError::NotBzImage);
        }
        Ok(h)
    }

    fn u8(&self, off: usize) -> u8 {
        self.image[off]
    }
    fn u16(&self, off: usize) -> u16 {
        u16::from_le_bytes(self.image[off..off + 2].try_into().unwrap())
    }
    fn u32(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.image[off..off + 4].try_into().unwrap())
    }
    fn u64(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.image[off..off + 8].try_into().unwrap())
    }

    fn version(&self) -> u16 {
        self.u16(hdr::VERSION)
    }

    /// setup header の範囲（boot_params にそのままコピーする）
    fn header_range(&self) -> Range<usize> {
        hdr::SETUP_SECTS..hdr::HEADER + self.u8(hdr::JUMP_END) as usize
    }

    /// bzImage のうち、保護モードのカーネル（setup の後ろ）が始まるオフセット
    fn payload_offset(&self) -> usize {
        let setup_sects = match self.u8(hdr::SETUP_SECTS) {
            0 => 4,
            n => n as usize,
        };
        (setup_sects + 1) * 512
    }

    fn payload(&self) -> &'a [u8] {
        &self.image[self.payload_offset()..]
    }
}

/// 起動データの領域への書き込み
struct BootData {
    base: u64,
}

impl BootData {
    fn write_bytes(&self, offset: u64, bytes: &[u8]) {
        // 物理メモリは HV の直接マップ（恒等マップ）で見える
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (self.base + offset) as *mut u8,
                bytes.len(),
            );
        }
    }
    fn write_u64(&self, offset: u64, value: u64) {
        self.write_bytes(offset, &value.to_le_bytes());
    }
    fn params_u8(&self, off: usize) -> u8 {
        unsafe { *((self.base + boot_data::BOOT_PARAMS + off as u64) as *const u8) }
    }
    fn params_bytes(&self, off: usize, bytes: &[u8]) {
        self.write_bytes(boot_data::BOOT_PARAMS + off as u64, bytes);
    }
    fn addr(&self, offset: u64) -> u64 {
        self.base + offset
    }
}

/// 管理 VM の初期状態
pub struct Prepared {
    pub config: GuestConfig,
    pub boot_params: u64,
}

/// カーネルを配置し、boot_params とゲスト用の GDT / ページテーブルを用意する
pub fn prepare(
    image: &LinuxImage,
    rsdp: u64,
    memory_map: &EfiMemoryMap,
    layout: &MemoryLayout,
) -> Result<Prepared, LinuxBootError> {
    let bz = unsafe { image.kernel.as_slice() };
    let header = SetupHeader::parse(bz)?;
    let cmdline = unsafe { image.cmdline.as_slice() };
    let cmdline = trim_cmdline(cmdline);

    // cmdline_size は終端の NUL を含まない最大長
    let cmdline_max = (header.u32(hdr::CMDLINE_SIZE) as usize).min(boot_data::CMDLINE_MAX - 1);
    if cmdline.len() > cmdline_max {
        return Err(LinuxBootError::CmdlineTooLong {
            len: cmdline.len(),
            max: cmdline_max,
        });
    }
    if image.initrd.is_present() {
        let end = image.initrd.address + image.initrd.size - 1;
        let max = header.u32(hdr::INITRD_ADDR_MAX) as u64;
        if end > max {
            return Err(LinuxBootError::InitrdAboveLimit { end, max });
        }
    }

    // ExitBootServices の後に空いているメモリから、HV のメモリを除いたもの。
    // bzImage / initrd / cmdline はブートローダが LoaderData に置いたので、ここには含まれない
    let mut free = mm::free_memory(memory_map, layout);

    let kernel = place_kernel(&header, &mut free)?;
    let boot_data_range = free
        .take(boot_data::SIZE, PAGE_SIZE_4K, PLACEMENT_MIN, PLACEMENT_MAX)
        .ok_or(LinuxBootError::NoSpaceForBootData)?;

    // カーネル本体（setup 部分は使わない）
    let payload = header.payload();
    assert!(payload.len() as u64 <= kernel.end - kernel.start);
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), kernel.start as *mut u8, payload.len());
    }

    let data = BootData {
        base: boot_data_range.start,
    };
    unsafe { core::ptr::write_bytes(data.base as *mut u8, 0, boot_data::SIZE as usize) };

    // boot_params: setup header をコピーし、ブートローダが書くフィールドを埋める
    let hr = header.header_range();
    data.params_bytes(hr.start, &bz[hr]);
    data.params_bytes(hdr::TYPE_OF_LOADER, &[hdr::LOADER_UNDEFINED]);
    let loadflags = data.params_u8(hdr::LOADFLAGS) | hdr::LOADED_HIGH;
    data.params_bytes(hdr::LOADFLAGS, &[loadflags]);
    data.params_bytes(hdr::CODE32_START, &(kernel.start as u32).to_le_bytes());

    data.write_bytes(boot_data::CMDLINE, cmdline);
    let cmdline_addr = data.addr(boot_data::CMDLINE);
    data.params_bytes(hdr::CMD_LINE_PTR, &(cmdline_addr as u32).to_le_bytes());
    data.params_bytes(
        params::EXT_CMD_LINE_PTR,
        &((cmdline_addr >> 32) as u32).to_le_bytes(),
    );

    if image.initrd.is_present() {
        let (addr, size) = (image.initrd.address, image.initrd.size);
        data.params_bytes(hdr::RAMDISK_IMAGE, &(addr as u32).to_le_bytes());
        data.params_bytes(hdr::RAMDISK_SIZE, &(size as u32).to_le_bytes());
        data.params_bytes(
            params::EXT_RAMDISK_IMAGE,
            &((addr >> 32) as u32).to_le_bytes(),
        );
        data.params_bytes(
            params::EXT_RAMDISK_SIZE,
            &((size >> 32) as u32).to_le_bytes(),
        );
    }

    // ACPI のテーブルはそのまま渡す（TODO(#10): HV が書き換えたものを渡すか検討する）。
    // EFI を使わない起動なので、RSDP の場所はここで教えないと見つけられない
    data.params_bytes(params::ACPI_RSDP_ADDR, &rsdp.to_le_bytes());

    let e820 = e820::build(memory_map, &layout.reserved());
    if e820.len() > e820::MAX_ENTRIES_ZEROPAGE {
        // TODO: SETUP_E820_EXT（setup_data）で残りを渡す
        return Err(LinuxBootError::TooManyE820Entries(e820.len()));
    }
    for (i, e) in e820.iter().enumerate() {
        let mut raw = [0u8; params::E820_ENTRY_SIZE];
        raw[0..8].copy_from_slice(&e.range.start.to_le_bytes());
        raw[8..16].copy_from_slice(&(e.range.end - e.range.start).to_le_bytes());
        raw[16..20].copy_from_slice(&e.kind.to_le_bytes());
        data.params_bytes(params::E820_TABLE + i * params::E820_ENTRY_SIZE, &raw);
    }
    data.params_bytes(params::E820_ENTRIES, &[e820.len() as u8]);

    // GDT と、下位 4GiB を 2MiB ページで恒等マップするページテーブル
    for (i, d) in GDT_ENTRIES.iter().enumerate() {
        data.write_u64(boot_data::GDT + i as u64 * 8, *d);
    }
    const PRESENT: u64 = 1 << 0;
    const WRITABLE: u64 = 1 << 1;
    const PAGE_SIZE: u64 = 1 << 7;
    data.write_u64(
        boot_data::PML4,
        data.addr(boot_data::PDPT) | PRESENT | WRITABLE,
    );
    for i in 0..boot_data::PD_COUNT {
        let pd = boot_data::PD + i * PAGE_SIZE_4K;
        data.write_u64(boot_data::PDPT + i * 8, data.addr(pd) | PRESENT | WRITABLE);
        for j in 0..512 {
            let phys = i * PAGE_SIZE_1G + j * PAGE_SIZE_2M;
            data.write_u64(pd + j * 8, phys | PRESENT | WRITABLE | PAGE_SIZE);
        }
    }

    println!(
        "[linux] protocol {}.{:02}, kernel {:#x}-{:#x}, boot data {:#x}-{:#x}, initrd {:#x}+{:#x}",
        header.version() >> 8,
        header.version() & 0xff,
        kernel.start,
        kernel.end,
        boot_data_range.start,
        boot_data_range.end,
        image.initrd.address,
        image.initrd.size
    );
    println!(
        "[linux] cmdline: {}",
        core::str::from_utf8(cmdline).unwrap_or("<non-UTF-8>")
    );
    for e in &e820 {
        println!(
            "[linux] e820: [{:#018x}-{:#018x}] {}",
            e.range.start,
            e.range.end - 1,
            e820::kind_name(e.kind)
        );
    }

    let gdtr = DescriptorTable {
        base: data.addr(boot_data::GDT),
        limit: (GDT_ENTRIES.len() * 8 - 1) as u16,
    };
    let config = GuestConfig::long_mode(
        kernel.start + ENTRY_64_OFFSET,
        data.addr(boot_data::STACK_TOP),
        data.addr(boot_data::PML4),
        gdtr,
        BOOT_CS,
        BOOT_DS,
    );
    Ok(Prepared {
        config,
        boot_params: data.addr(boot_data::BOOT_PARAMS),
    })
}

/// カーネルのロード先を決める。pref_address が空いていればそこを、なければ空いている場所を使う
fn place_kernel(header: &SetupHeader, free: &mut RangeSet) -> Result<Range<u64>, LinuxBootError> {
    let init_size = header.u32(hdr::INIT_SIZE) as u64;
    let pref = header.u64(hdr::PREF_ADDRESS);
    let preferred = pref..pref + init_size;
    if free.contains_range(&preferred) {
        free.remove(&preferred);
        return Ok(preferred);
    }
    if header.u8(hdr::RELOCATABLE_KERNEL) == 0 {
        return Err(LinuxBootError::NoSpaceForKernel { size: init_size });
    }
    let align = (header.u32(hdr::KERNEL_ALIGNMENT) as u64).max(PAGE_SIZE_4K);
    free.take(init_size, align, PLACEMENT_MIN, PLACEMENT_MAX)
        .ok_or(LinuxBootError::NoSpaceForKernel { size: init_size })
}

/// cmdline.txt の末尾の改行や NUL を取り除く
fn trim_cmdline(cmdline: &[u8]) -> &[u8] {
    let end = cmdline
        .iter()
        .rposition(|&c| !matches!(c, b'\n' | b'\r' | b' ' | b'\t' | 0))
        .map_or(0, |i| i + 1);
    let cmdline = &cmdline[..end];
    // 途中に NUL があればそこまで
    let nul = cmdline
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(cmdline.len());
    &cmdline[..nul]
}

/// 管理 VM を起動して実行し続ける。ゲストが止まったら理由を返す
pub fn boot(
    image: &LinuxImage,
    rsdp: u64,
    memory_map: &EfiMemoryMap,
    layout: &MemoryLayout,
) -> Result<StopReason, LinuxBootError> {
    let prepared = prepare(image, rsdp, memory_map, layout)?;
    let ept = ept::management_vm(layout)
        .expect("EPT is not supported")
        .into_shared();
    // AP は管理 VM の vCPU として、Linux が SIPI を送るのを待つ
    smp::MANAGEMENT_VM_EPT.call_once(|| ept);
    smp::wait_for_aps_in_guest();
    let mut vcpu = Vcpu::new(&prepared.config, ept);
    // 64bit boot protocol: RSI に boot_params の物理アドレス
    vcpu.regs.rsi = prepared.boot_params;
    println!(
        "[linux] entering the kernel at {:#x} (boot_params {:#x})",
        prepared.config.rip, prepared.boot_params
    );
    Ok(vcpu.run())
}
