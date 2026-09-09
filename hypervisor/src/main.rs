#![no_std]
#![no_main]

use core::alloc::{GlobalAlloc, Layout};
use console::println;
use serial::putc;
use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::NonNull;
use acpi::*;

use spin::Mutex;

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
    let mut i = 0;
    loop {
        let desc_ptr = (boot_info.map_buffer as usize + i * boot_info.descriptor_size) as *mut EfiMemoryDescriptor;
        if desc_ptr as usize > boot_info.map_buffer as usize + boot_info.map_size {
            break;
        }
        unsafe {
            let desc =  &*desc_ptr;
            if desc.memory_type == EfiMemoryType::ConventionalMemory as u32 ||
                desc.memory_type == EfiMemoryType::BootServicesCode as u32 ||
                desc.memory_type == EfiMemoryType::BootServicesData as u32
            {
                // 0番地からスタートするエントリはスキップ
                if desc.physical_start == 0 {
                    continue;
                }

                MEMORY_ALLOCATOR
                    .lock()
                    .add_to_heap(desc.physical_start as usize, desc.number_of_pages as usize * 4096);
            }
        }
        i += 1;
    }

    *console::CONSOLE.lock() = Some(console::Console::new(putc));
    println!("hello world from hypervisor.");
        
    let xsdt = unsafe {
        &*(boot_info.acpi_xsdt_address as *const Xsdt)
    };
    if !is_valid_signature(xsdt) {
        halt_loop();
    }
    if !calclate_checksum(xsdt) {
        halt_loop();
    }

    let entry_num = (xsdt.header.length as usize - core::mem::size_of::<AcpiTableHeader>()) / core::mem::size_of::<*const u64>();
    let entries = unsafe {
        &*core::slice::from_raw_parts(xsdt.entry as *const AcpiTableHeader, entry_num)
    };

    for entry in entries {
        if entry.signature == "FADT".as_bytes() {
        }
    }

    halt_loop();
}

fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    halt_loop();
}
