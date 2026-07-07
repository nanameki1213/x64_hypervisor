use console::println;

use crate::{BootServices, uefi};

#[repr(C)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64, // オフセット 0x18
    pub e_phoff: u64, // プログラムヘッダテーブルのオフセット
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16, // プログラムヘッダの数
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64, // ファイル内オフセット
    pub p_vaddr: u64,
    pub p_paddr: u64,  // 物理アドレス
    pub p_filesz: u64, // ファイル上のサイズ
    pub p_memsz: u64,  // メモリ上のサイズ（p_filesz以上、差分が.bss）
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;

pub fn load_elf(base_address: *const u64, bs: &BootServices) -> u64 {
    unsafe {
        let ehdr = base_address as *const Elf64Ehdr;
        let e_phoff = (*ehdr).e_phoff;
        let e_phnum = (*ehdr).e_phnum;
        let e_entry = (*ehdr).e_entry;

        let mut physical_entry = 0;

        let mut total_size = 0;
        let mut flag = false;
        let mut min_lma = 0;
        for i in 0..e_phnum {
            let phdr = (base_address as u64 + e_phoff + i as u64 * 56) as *const Elf64Phdr;
            if (*phdr).p_type != PT_LOAD {
                continue;
            }

            total_size += (*phdr).p_memsz as usize;
            if !flag {
                min_lma = (*phdr).p_paddr;
                flag = true;
            }
        }

        let load_address = match bs.allocate_pages_at(
            uefi::EfiMemoryType::LoaderCode,
            total_size.div_ceil(0x1000),
            min_lma,
        ) {
            Ok(addr) => addr,
            Err(err) => panic!("failed to allocate memory: {:?}", err),
        };

        println!("load_address: {:#x}", load_address);

        for i in 0..e_phnum {
            let phdr = (base_address as u64 + e_phoff + i as u64 * 56) as *const Elf64Phdr;
            if (*phdr).p_type != PT_LOAD {
                continue;
            }

            let physical_address = load_address + ((*phdr).p_paddr - min_lma);

            core::ptr::copy(
                (base_address as u64 + (*phdr).p_offset) as *const u8,
                physical_address as *mut u8,
                (*phdr).p_filesz as usize,
            );

            let zero_start = physical_address + (*phdr).p_filesz;
            let zero_size = (*phdr).p_memsz - (*phdr).p_filesz;
            core::ptr::write_bytes(zero_start as *mut u8, 0, zero_size as usize);

            if e_entry >= (*phdr).p_vaddr && e_entry < (*phdr).p_vaddr + (*phdr).p_memsz {
                physical_entry = physical_address + (e_entry - (*phdr).p_vaddr);
            }
        }

        physical_entry
    }
}
