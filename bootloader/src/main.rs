#![no_std]
#![no_main]

use crate::uefi::{EfiStatus, EfiSystemTable};

mod elf;
mod uefi;

use console::print;
use core::panic::PanicInfo;
use core::arch::asm;
use core::ffi::c_void;
use spin::Once;
use uefi::*;

pub static BOOT_SERVICE: Once<&'static EfiBootServices> = Once::new();
pub static CON_OUT: Once<&'static EfiSimpleTextOutputProtocol> = Once::new();

pub struct MemoryMap {
    pub buffer: *mut u8,
    pub map_size: usize,
    pub map_key: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}

#[derive(Debug)]
pub struct UefiError(pub EfiStatus);

pub struct MemoryMapSizeInfo {
    pub required_size: usize,
    pub descriptor_size: usize,
}

pub fn efi_status_to_result(status: EfiStatus) -> Result<(), UefiError> {
    if EfiStatus::is_error(status) {
        Err(UefiError(status))
    } else if EfiStatus::is_warning(status) {
        Err(UefiError(status))
    } else {
        Ok(())
    }
}

pub struct BootServices {
    bs: &'static EfiBootServices,
}

impl BootServices {
    pub const fn new(bs: &'static EfiBootServices) -> Self {
        Self { bs }
    }

    pub fn allocate_any_pages(
        &self,
        memory_type: EfiMemoryType,
        pages: usize,
    ) -> Result<EfiPhysicalAddress, UefiError> {
        let mut addr: EfiPhysicalAddress = 0;
        let status = unsafe {
            (self.bs.allocate_pages)(
                EfiAllocateType::AllocateAnyPages,
                memory_type,
                pages,
                &mut addr,
            )
        };
        efi_status_to_result(status)?;
        Ok(addr)
    }

    pub fn allocate_pages_below(
        &self,
        memory_type: EfiMemoryType,
        pages: usize,
        max_addr: EfiPhysicalAddress,
    ) -> Result<EfiPhysicalAddress, UefiError> {
        let mut addr: EfiPhysicalAddress = max_addr;
        let status = unsafe {
            (self.bs.allocate_pages)(
                EfiAllocateType::AllocateMaxAddress,
                memory_type,
                pages,
                &mut addr,
            )
        };
        efi_status_to_result(status)?;
        Ok(addr)
    }

    pub fn allocate_pages_at(
        &self,
        memory_type: EfiMemoryType,
        pages: usize,
        addr: EfiPhysicalAddress,
    ) -> Result<(), UefiError> {
        let mut address = addr;
        let status = unsafe {
            (self.bs.allocate_pages)(
                EfiAllocateType::AllocateAddress,
                memory_type,
                pages,
                &mut address,
            )
        };
        efi_status_to_result(status)?;
        Ok(())
    }

    pub fn get_memory_map<'a>(&self) -> Result<MemoryMap, UefiError> {
        let mut map_size = 0;
        let mut map_key = 0;
        let mut descriptor_size = 0;
        let mut descriptor_version = 0;
        unsafe {
            (self.bs.get_memory_map)(
                &mut map_size,
                core::ptr::null_mut(),
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            )
        };

        map_size += descriptor_size * 2;
        let memmap_buf_ptr = self.allocate_pool(EfiMemoryType::LoaderData, map_size)?;

        let status = unsafe {
            (self.bs.get_memory_map)(
                &mut map_size,
                memmap_buf_ptr as *mut EfiMemoryDescriptor,
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            )
        };
        efi_status_to_result(status)?;

        Ok(MemoryMap {
            buffer: memmap_buf_ptr,
            map_size,
            map_key,
            descriptor_size,
            descriptor_version,
        })
    }

    pub fn get_memory_map_with_buf<'a>(&self, buffer: &'a [u8]) -> Result<MemoryMap, UefiError> {
        let mut map_size = buffer.len();
        let mut map_key = 0;
        let mut descriptor_size = 0;
        let mut descriptor_version = 0;

        let status = unsafe {
            (self.bs.get_memory_map)(
                &mut map_size,
                buffer.as_ptr() as *mut EfiMemoryDescriptor,
                &mut map_key,
                &mut descriptor_size,
                &mut descriptor_version,
            )
        };
        efi_status_to_result(status)?;
        Ok(MemoryMap {
            map_size,
            buffer: buffer.as_ptr() as *mut u8,
            map_key,
            descriptor_size,
            descriptor_version,
        })
    }

    pub fn allocate_pool(
        &self,
        memory_type: EfiMemoryType,
        size: usize,
    ) -> Result<*mut u8, UefiError> {
        let mut buf: *mut c_void = core::ptr::null_mut();
        let status = unsafe { (self.bs.allocate_pool)(memory_type, size, &mut buf) };
        efi_status_to_result(status)?;
        Ok(buf as *mut u8)
    }

    pub fn free_pool(&self, buffer: *mut c_void) -> Result<(), UefiError> {
        let status = unsafe { (self.bs.free_pool)(buffer) };
        efi_status_to_result(status)?;
        Ok(())
    }

     pub fn exit_boot_services(
        &self,
        image_handle: EfiHandle,
        map_key: usize,
    ) -> Result<(), UefiError> {
        let status = unsafe { (self.bs.exit_boot_services)(image_handle, map_key) };
        efi_status_to_result(status)?;
        Ok(())
    }
}

fn boot_services() -> &'static EfiBootServices {
    *BOOT_SERVICE.get().expect("Boot Services not initialized")
}

fn con_out() -> &'static EfiSimpleTextOutputProtocol {
    *CON_OUT.get().expect("ConOut not initialized")
}

/// UEFI コンソール向け write_byte。
/// `\n` を `\r\n` に変換し、1 バイトを UTF-16 に変換して output_string で出力する。
fn uefi_putc(b: u8) {
    let out = con_out();
    if b == b'\n' {
        let buf = [b'\r' as u16, b'\n' as u16, 0u16];
        unsafe { (out.output_string)(out as *const _ as *mut _, buf.as_ptr()) };
    } else {
        let buf = [b as u16, 0u16];
        unsafe { (out.output_string)(out as *const _ as *mut _, buf.as_ptr()) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "efiapi" fn efi_main(
    image_handle: EfiHandle,
    system_table: *mut EfiSystemTable,
) -> EfiStatus {
    let table = unsafe { &*system_table };

    let boot_service = unsafe { &*table.boot_services };
    BOOT_SERVICE.call_once(|| boot_service);
    let bs = BootServices::new(boot_services());

    let con_out_proto = unsafe { &*table.con_out };
    CON_OUT.call_once(|| con_out_proto);
    *console::CONSOLE.lock() = Some(console::Console::new(uefi_putc));

    print!("Hello, World!\r\n");

    //halt_loop();

    return EfiStatus::Success;
}

fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    print!("{}\r\n", info);
    halt_loop();
}
