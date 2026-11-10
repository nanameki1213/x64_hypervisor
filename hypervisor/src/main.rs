#![no_std]
#![no_main]

extern crate alloc;

use acpi::*;
use boot_info::BootInfo;
use console::println;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::arch::global_asm;
use core::panic::PanicInfo;
use core::ptr::NonNull;

use spin::Mutex;

mod arch;
mod linux;
mod mm;

/// HV のログ出力先。COM1 は管理 VM のコンソールに譲る
const HV_LOG_PORT: u16 = serial::COM2;

/// 起動テスト（`cargo xtask test`）が成功判定に使う文字列
const BOOT_OK_MARKER: &str = "[hypervisor] boot ok";
/// CPU が VMX に対応していないときに出す文字列。
/// nested VMX が使えない CI では、`cargo xtask test --allow-no-vmx` がこれを成功とみなす
const VMX_UNAVAILABLE_MARKER: &str = "[hypervisor] VMX is not available on this CPU";
/// 管理 VM が止まったときに出す文字列。起動テストはこれを失敗とみなす
pub const GUEST_STOPPED_MARKER: &str = "[hypervisor] management VM stopped";

fn hv_putc(c: u8) {
    serial::write_byte(HV_LOG_PORT, c);
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
    // BootInfo はブートローダのスタック（BootServicesData）上にあり、
    // その領域をヒープとして再利用すると壊れるので、最初にコピーしておく
    let boot_info = *boot_info;

    serial::init(HV_LOG_PORT);
    *console::CONSOLE.lock() = Some(console::Console::new(hv_putc));
    println!("hello world from hypervisor.");

    arch::x86_64::init();

    let memory_map = unsafe {
        mm::EfiMemoryMap::new(
            boot_info.map_buffer as *const u8,
            boot_info.map_size as usize,
            boot_info.descriptor_size as usize,
        )
    };
    let layout = mm::init(&memory_map);
    println!(
        "[mm] image {:#x}-{:#x}, heap {:#x}-{:#x}, memory map top {:#x}",
        layout.image.start,
        layout.image.end,
        layout.heap.start,
        layout.heap.end,
        layout.memory_map_top
    );
    MEMORY_ALLOCATOR.lock().add_to_heap(
        layout.heap.start as usize,
        (layout.heap.end - layout.heap.start) as usize,
    );

    let xsdt = unsafe { &*(boot_info.acpi_xsdt_address as *const Xsdt) };
    if !is_valid_signature(xsdt) {
        halt_loop();
    }
    if !calclate_checksum(xsdt) {
        halt_loop();
    }

    let cpus = unsafe { arch::x86_64::smp::discover(xsdt) };
    println!(
        "[smp] {} CPUs (APIC IDs {:?}), BSP APIC ID {}",
        cpus.apic_ids.len(),
        cpus.apic_ids,
        cpus.bsp_apic_id
    );
    let online = unsafe { arch::x86_64::smp::start_aps(&cpus, xsdt, &memory_map, &layout) };
    println!("[smp] {} of {} APs online", online, cpus.apic_ids.len() - 1);

    if let Err(e) = arch::x86_64::vmx::enable() {
        if matches!(e, arch::x86_64::vmx::VmxInitError::NotSupported) {
            println!("{}", VMX_UNAVAILABLE_MARKER);
        }
        panic!("failed to enable VMX: {:?}", e);
    }
    arch::x86_64::vmx::selftest::run(&layout);

    println!("{}", BOOT_OK_MARKER);

    if !boot_info.linux.kernel.is_present() {
        println!(
            "[hypervisor] no Linux kernel (bzImage) on the ESP; run `cargo xtask linux` to build one"
        );
        halt_loop();
    }
    match linux::boot(
        &boot_info.linux,
        boot_info.acpi_rsdp_address,
        &memory_map,
        &layout,
    ) {
        Ok(reason) => println!("{}: {:?}", GUEST_STOPPED_MARKER, reason),
        Err(e) => panic!("failed to boot the management VM: {:?}", e),
    }
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
