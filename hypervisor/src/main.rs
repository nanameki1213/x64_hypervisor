#![no_std]
#![no_main]

use acpi::*;
use console::println;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::arch::global_asm;
use core::panic::PanicInfo;
use core::ptr::NonNull;

use spin::Mutex;

mod arch;

/// HV のログ出力先。COM1 は管理 VM のコンソールに譲る
const HV_LOG_PORT: u16 = serial::COM2;

/// 起動テスト（`cargo xtask test`）が成功判定に使う文字列
const BOOT_OK_MARKER: &str = "[hypervisor] boot ok";

fn hv_putc(c: u8) {
    serial::write_byte(HV_LOG_PORT, c);
}

pub struct BootInfo {
    pub acpi_xsdt_address: *const u8,
    pub map_buffer: *const u8,
    pub map_size: usize,
    pub descriptor_size: usize,
}

/// EFI_MEMORY_DESCRIPTOR
#[repr(C)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

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

global_asm!(include_str!("boot.S"), options(att_syntax));

static MEMORY_ALLOCATOR: Mutex<allocator::Heap<33>> = Mutex::new(allocator::Heap::new());

struct GlobalAllocator {}

#[global_allocator]
static GLOBAL_ALLOCATOR: GlobalAllocator = GlobalAllocator {};

unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match MEMORY_ALLOCATOR.lock().allocate(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match MEMORY_ALLOCATOR.lock().callocate(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            MEMORY_ALLOCATOR
                .lock()
                .deallocate(NonNull::new_unchecked(ptr), layout);
        }
    }
}

#[unsafe(no_mangle)]
extern "win64" fn main(boot_info: &BootInfo) {
    serial::init(HV_LOG_PORT);
    *console::CONSOLE.lock() = Some(console::Console::new(hv_putc));
    println!("hello world from hypervisor.");

    arch::x86_64::init();

    let num_descs = boot_info.map_size / boot_info.descriptor_size;
    for i in 0..num_descs {
        let desc_ptr = (boot_info.map_buffer as usize + i * boot_info.descriptor_size)
            as *const EfiMemoryDescriptor;
        unsafe {
            let desc = &*desc_ptr;
            if desc.memory_type == EfiMemoryType::ConventionalMemory as u32
                || desc.memory_type == EfiMemoryType::BootServicesCode as u32
                || desc.memory_type == EfiMemoryType::BootServicesData as u32
            {
                // 0番地からスタートするエントリはスキップ
                if desc.physical_start == 0 {
                    continue;
                }

                MEMORY_ALLOCATOR.lock().add_to_heap(
                    desc.physical_start as usize,
                    desc.number_of_pages as usize * 4096,
                );
            }
        }
    }

    let xsdt = unsafe { &*(boot_info.acpi_xsdt_address as *const Xsdt) };
    if !is_valid_signature(xsdt) {
        halt_loop();
    }
    if !calclate_checksum(xsdt) {
        halt_loop();
    }

    let entry_num = (xsdt.header.length as usize - core::mem::size_of::<AcpiTableHeader>())
        / core::mem::size_of::<*const u64>();
    let entries =
        unsafe { core::slice::from_raw_parts(xsdt.entry as *const AcpiTableHeader, entry_num) };

    for entry in entries {
        if entry.signature == "FADT".as_bytes() {}
    }

    println!("{}", BOOT_OK_MARKER);
    halt_loop();
}

fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    // panic がコンソールのロック中に起きた可能性があるため、強制的に解放する
    unsafe { console::CONSOLE.force_unlock() };
    println!("[PANIC] {}", info);
    halt_loop();
}
