#![no_std]
#![no_main]

use crate::uefi::{EfiStatus, EfiSystemTable};

mod elf;
mod uefi;

use acpi::*;
use boot_info::{BootInfo, LinuxImage, LoadedFile};
use console::print;
use core::arch::asm;
use core::ffi::c_void;
use core::panic::PanicInfo;
use elf::*;
use spin::Once;
use uefi::*;

pub const KERNEL_ALIGN: usize = 0x200000;

pub static BOOT_SERVICE: Once<&'static EfiBootServices> = Once::new();
pub static CON_OUT: Once<&'static EfiSimpleTextOutputProtocol> = Once::new();

/// ESP のルートに置くファイル（cargo xtask build / cargo xtask linux が配置する）
const HYPERVISOR_FILE: &str = "hypervisor.elf";
const LINUX_KERNEL_FILE: &str = "bzImage";
const LINUX_INITRD_FILE: &str = "initrd.img";
const LINUX_CMDLINE_FILE: &str = "cmdline.txt";

/// HV が管理 VM の起動前にアクセスするので、ファイルは 4GiB 未満に読み込む
const LOAD_MAX_ADDRESS: EfiPhysicalAddress = 0xffff_ffff;

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
    if EfiStatus::is_error(status) || EfiStatus::is_warning(status) {
        Err(UefiError(status))
    } else {
        Ok(())
    }
}

pub struct BootServices {
    bs: &'static EfiBootServices,
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
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
    ) -> Result<EfiPhysicalAddress, UefiError> {
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
        Ok(address)
    }

    pub fn free_pages(&self, memory: EfiPhysicalAddress, pages: usize) -> Result<(), UefiError> {
        let status = unsafe { (self.bs.free_pages)(memory, pages) };
        efi_status_to_result(status)?;
        Ok(())
    }

    pub fn get_memory_map(&self) -> Result<MemoryMap, UefiError> {
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

    pub fn get_memory_map_with_buf(&self, buffer: &[u8]) -> Result<MemoryMap, UefiError> {
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

    pub fn locate_handle_buffer(
        &self,
        search_type: EfiLocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        no_handles: *mut usize,
    ) -> Result<*mut EfiHandle, UefiError> {
        let mut buffer: *mut EfiHandle = core::ptr::null_mut();
        let status = unsafe {
            (self.bs.locate_handle_buffer)(
                search_type,
                protocol,
                search_key,
                no_handles,
                &raw mut buffer,
            )
        };
        efi_status_to_result(status)?;
        Ok(buffer)
    }

    pub fn open_protocol(
        &self,
        image_handle: EfiHandle,
        protocol: *const EfiGuid,
        agent_handle: EfiHandle,
        controller_handle: EfiHandle,
        attributes: u32,
    ) -> Result<*mut c_void, UefiError> {
        let mut interface: *mut c_void = core::ptr::null_mut();
        let status = unsafe {
            (self.bs.open_protocol)(
                image_handle,
                protocol,
                &raw mut interface,
                agent_handle,
                controller_handle,
                attributes,
            )
        };
        efi_status_to_result(status)?;
        Ok(interface)
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
    BOOT_SERVICE.get().expect("Boot Services not initialized")
}

fn con_out() -> &'static EfiSimpleTextOutputProtocol {
    CON_OUT.get().expect("ConOut not initialized")
}

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

/// UEFIアプリケーションのエントリポイント
///
/// # Safety
///
/// この関数はUEFIファームウェアから直接呼び出されます。
/// * `image_handle`: ファームウェアによって生成された有効なイメージハンドル
/// * `system_table`: ファームウェアが提供する有効な`EfiSystemTable`を指す
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
    {
        *console::CONSOLE.lock() = Some(console::Console::new(uefi_putc));
    }

    let root_dir = match open_root_dir(&bs, image_handle) {
        Ok(root) => root,
        Err(err) => panic!("failed to open root directory: {:?}", err),
    };

    let hypervisor = match load_file(&bs, root_dir, HYPERVISOR_FILE) {
        Ok(Some(file)) => file,
        Ok(None) => panic!("{} is not found on the ESP", HYPERVISOR_FILE),
        Err(err) => panic!("failed to load {}: {:?}", HYPERVISOR_FILE, err),
    };
    let load_optional = |name: &str| match load_file(&bs, root_dir, name) {
        Ok(Some(file)) => {
            print!(
                "loaded {} at {:#x} ({} bytes)\r\n",
                name, file.address, file.size
            );
            file
        }
        Ok(None) => LoadedFile::NONE,
        Err(err) => panic!("failed to load {}: {:?}", name, err),
    };
    let linux = LinuxImage {
        kernel: load_optional(LINUX_KERNEL_FILE),
        initrd: load_optional(LINUX_INITRD_FILE),
        cmdline: load_optional(LINUX_CMDLINE_FILE),
    };

    let entry_address = load_elf(hypervisor.address as *const u64, &bs);

    let config_table = unsafe {
        core::slice::from_raw_parts(
            (*system_table).configuration_table,
            (*system_table).number_of_table_entries,
        )
    };

    let mut rsdp_address = 0;
    let mut xsdt_address = 0;
    for config in config_table {
        if config.vendor_guid == EFI_ACPI_RSDP_STRUCTURE_GUID {
            let rsdp = unsafe { &*(config.vendor_table as *const RootSystemDescriptionPointer) };
            if !unsafe {
                root_system_description_pointer_verification(
                    config.vendor_table as *const RootSystemDescriptionPointer,
                )
            } {
                panic!("RSDP is invalid!");
            }
            rsdp_address = config.vendor_table as u64;
            xsdt_address = rsdp.xsdt_address;
        }
    }

    let memmap = match bs.get_memory_map() {
        Ok(map) => map,
        Err(err) => panic!("failed to get memory map: {:?}", err),
    };

    match bs.exit_boot_services(image_handle, memmap.map_key) {
        Ok(_) => {}
        Err(_) => halt_loop(),
    };

    let boot_info = BootInfo {
        acpi_rsdp_address: rsdp_address,
        acpi_xsdt_address: xsdt_address,
        map_buffer: memmap.buffer as u64,
        map_size: memmap.map_size as u64,
        descriptor_size: memmap.descriptor_size as u64,
        linux,
    };

    let entry_point: unsafe extern "C" fn(boot_info: &BootInfo) =
        unsafe { core::mem::transmute(entry_address as usize) };
    unsafe {
        entry_point(&boot_info);
    }

    EfiStatus::Success
}

fn open_root_dir(
    bs: &BootServices,
    image_handle: EfiHandle,
) -> Result<*mut EfiFileProtocol, UefiError> {
    let loaded_image = bs.open_protocol(
        image_handle,
        &raw const EFI_LOADED_IMAGE_PROTOCOL_GUID,
        image_handle,
        core::ptr::null_mut(),
        open_protocol::BY_HANDLE_PROTOCOL,
    )? as *mut EfiLoadedImageProtocol;
    let fs = unsafe {
        bs.open_protocol(
            (*loaded_image).device_handle,
            &raw const EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
            image_handle,
            core::ptr::null_mut(),
            open_protocol::BY_HANDLE_PROTOCOL,
        )? as *mut EfiSimpleFileSystemProtocol
    };

    let mut root: *mut EfiFileProtocol = core::ptr::null_mut();
    let status = unsafe { ((*fs).open_volume)(fs, &raw mut root) };
    efi_status_to_result(status)?;
    Ok(root)
}

/// ESP のルートにある `name` を、4GiB 未満の LoaderData のページに読み込む。
/// ファイルがない、または空のときは None を返す
fn load_file(
    bs: &BootServices,
    root: *mut EfiFileProtocol,
    name: &str,
) -> Result<Option<LoadedFile>, UefiError> {
    let mut name16 = [0u16; 64];
    assert!(name.len() < name16.len(), "file name is too long: {}", name);
    for (i, c) in name.encode_utf16().enumerate() {
        name16[i] = c;
    }

    let mut file: *mut EfiFileProtocol = core::ptr::null_mut();
    let status =
        unsafe { ((*root).open)(root, &raw mut file, name16.as_ptr(), open_mode::READ, 0) };
    if status == EfiStatus::NotFound {
        return Ok(None);
    }
    efi_status_to_result(status)?;

    let result = (|| {
        // EFI_FILE_INFO の後ろにファイル名が続くので、余裕を持たせる
        let mut info = [0u64; 64];
        let mut info_size = core::mem::size_of_val(&info);
        let status = unsafe {
            ((*file).get_info)(
                file,
                &raw const EFI_FILE_INFO_ID,
                &raw mut info_size,
                info.as_mut_ptr() as *mut c_void,
            )
        };
        efi_status_to_result(status)?;
        let file_size = unsafe { (*(info.as_ptr() as *const EfiFileInfo)).file_size } as usize;
        if file_size == 0 {
            return Ok(None);
        }

        let address = bs.allocate_pages_below(
            EfiMemoryType::LoaderData,
            file_size.div_ceil(EFI_PAGE_SIZE),
            LOAD_MAX_ADDRESS,
        )?;
        let mut read_size = file_size;
        let status = unsafe { ((*file).read)(file, &raw mut read_size, address as *mut c_void) };
        efi_status_to_result(status)?;
        if read_size != file_size {
            return Err(UefiError(EfiStatus::DeviceError));
        }
        Ok(Some(LoadedFile {
            address,
            size: file_size as u64,
        }))
    })();
    unsafe { ((*file).close)(file) };
    result
}

fn allocate_pages_aligned(
    bs: &BootServices,
    memory_type: EfiMemoryType,
    pages: usize,
    mut align: usize,
) -> Result<EfiPhysicalAddress, UefiError> {
    if align < EFI_ALLOC_ALIGN {
        align = EFI_ALLOC_ALIGN;
    }

    let slack = align / EFI_PAGE_SIZE - 1;

    let addr = bs.allocate_any_pages(memory_type, pages + slack)?;

    let aligned = (addr as usize + align - 1) & !(align - 1);
    if slack > 0 {
        let head = (aligned - addr as usize) / EFI_PAGE_SIZE;
        let tail = slack - head;
        if head > 0 {
            bs.free_pages(addr, head)?;
        }
        if tail > 0 {
            bs.free_pages((aligned + (pages * EFI_PAGE_SIZE)) as u64, tail)?;
        }
    }
    Ok(aligned as u64)
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
