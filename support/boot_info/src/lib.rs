//! ブートローダから HV に渡す情報。
//!
//! ブートローダ（x86_64-unknown-uefi）と HV（x86_64-unknown-none）は別々にビルドされるので、
//! レイアウトが一致するよう `repr(C)` にして、このクレートで共有する。

#![no_std]

/// ESP から読み込んだファイル。読み込まなかった（ファイルがなかった）ときは `size == 0`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LoadedFile {
    /// 先頭の物理アドレス（4KiB アライン、4GiB 未満、メモリタイプは LoaderData）
    pub address: u64,
    pub size: u64,
}

impl LoadedFile {
    pub const NONE: LoadedFile = LoadedFile {
        address: 0,
        size: 0,
    };

    pub fn is_present(&self) -> bool {
        self.size != 0
    }

    /// # Safety
    /// ブートローダが読み込んだ領域が、まだ上書きされていないこと
    pub unsafe fn as_slice(&self) -> &'static [u8] {
        if !self.is_present() {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.address as *const u8, self.size as usize) }
    }
}

/// 管理 VM（Linux）のイメージ
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LinuxImage {
    /// `bzImage`
    pub kernel: LoadedFile,
    /// `initrd.img`
    pub initrd: LoadedFile,
    /// `cmdline.txt`（NUL 終端なし）
    pub cmdline: LoadedFile,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BootInfo {
    pub acpi_rsdp_address: u64,
    pub acpi_xsdt_address: u64,
    /// `ExitBootServices` に渡したメモリマップ（LoaderData）
    pub map_buffer: u64,
    pub map_size: u64,
    pub descriptor_size: u64,
    pub linux: LinuxImage,
}
