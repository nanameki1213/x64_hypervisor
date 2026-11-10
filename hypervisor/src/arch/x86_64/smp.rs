//! マルチプロセッサ（#8）。vCPU と物理 CPU は 1:1 で固定する

use acpi::{Fadt, Madt, Xsdt};
use alloc::alloc::Layout;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use console::println;
use spin::{Mutex, Once};

use super::vmx::{
    self,
    ept::Ept,
    vcpu::{GuestConfig, Vcpu},
};
use super::{cpu, gdt, idt};
use crate::mm::{self, EfiMemoryMap, MemoryLayout, PAGE_SIZE_4K};

/// 起動時に MADT から調べた CPU の構成
#[derive(Debug)]
pub struct CpuTopology {
    /// 使える CPU の APIC ID（MADT の順。BSP を含む）
    pub apic_ids: Vec<u32>,
    /// BSP（この関数を実行している CPU）の APIC ID
    pub bsp_apic_id: u32,
}

/// 現在の CPU の APIC ID。x2APIC の ID（CPUID 0xB）が取れればそちらを使う
pub fn current_apic_id() -> u32 {
    if cpu::cpuid(0, 0).eax >= 0xb {
        let r = cpu::cpuid(0xb, 0);
        // サブリーフ 0 の EBX が 0 なら leaf 0xB は無効
        if r.ebx != 0 {
            return r.edx;
        }
    }
    cpu::cpuid(1, 0).ebx >> 24
}

/// # Safety
/// `xsdt` のエントリが、読めるメモリ上の ACPI テーブルを指していること
pub unsafe fn discover(xsdt: &Xsdt) -> CpuTopology {
    let madt = unsafe { xsdt.find::<Madt>() }.expect("MADT is not found in the XSDT");
    let topology = CpuTopology {
        apic_ids: madt.apic_ids().collect(),
        bsp_apic_id: current_apic_id(),
    };
    assert!(
        topology.apic_ids.contains(&topology.bsp_apic_id),
        "the BSP (APIC ID {}) is not listed in the MADT",
        topology.bsp_apic_id
    );
    topology
}

// ---- AP の起動 ----
//
// BSP が低位 1MiB の空きページにトランポリンを置き、INIT-SIPI-SIPI で AP を 1 つずつ起こす。
// AP はリアルモードからトランポリンで 64bit モードに入り（ページテーブルは HV のものを共有する）、
// `ap_entry` で自分の GDT / TSS を読み込む。

/// AP 1 つあたりのスタック
const AP_STACK_SIZE: usize = 0x10000;
/// SIPI を送ってから AP の起動完了を待つ時間
const AP_START_TIMEOUT_US: u64 = 100_000;

/// トランポリンのページ内で、BSP が書き込むデータの位置（コードはこれより前に収める）
const TRAMPOLINE_DATA: usize = 0xf00;
/// データ内のオフセット（トランポリンのアセンブリと一致させる）
const TD_GDT: usize = 0x00; // 4 エントリ
const TD_GDTR: usize = 0x20; // limit (u16), base (u32)
const TD_PROT32: usize = 0x28; // far pointer: offset (u32), selector (u16)
const TD_LONG64: usize = 0x30; // far pointer: offset (u32), selector (u16)
const TD_CR3: usize = 0x38;
const TD_STACK: usize = 0x40;
const TD_ENTRY: usize = 0x48;
const TD_ARG: usize = 0x50;

/// トランポリン用の GDT（null, 32bit コード, データ, 64bit コード）
const TRAMPOLINE_GDT: [u64; 4] = [
    0,
    0x00cf_9a00_0000_ffff,
    0x00cf_9200_0000_ffff,
    0x00af_9a00_0000_ffff,
];
const TRAMPOLINE_CS32: u16 = 0x08;
const TRAMPOLINE_CS64: u16 = 0x18;

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_prot32: u8;
    static ap_trampoline_long64: u8;
    static ap_trampoline_end: u8;
}

// 位置に依存しないように書く。トランポリンの先頭の物理アドレスは CS から求めて EBX に置く
global_asm!(
    r#"
.set TD,        0xf00
.set TD_GDTR,   TD + 0x20
.set TD_PROT32, TD + 0x28
.set TD_LONG64, TD + 0x30
.set TD_CR3,    TD + 0x38
.set TD_STACK,  TD + 0x40
.set TD_ENTRY,  TD + 0x48
.set TD_ARG,    TD + 0x50

.section .text
.code16
.global ap_trampoline_start
ap_trampoline_start:
    cli
    cld
    mov   %cs, %bx
    mov   %bx, %ds
    movzwl %bx, %ebx
    shl   $4, %ebx
    lgdtl TD_GDTR
    mov   %cr0, %eax
    or    $1, %eax              # PE
    mov   %eax, %cr0
    ljmpl *TD_PROT32

.code32
.global ap_trampoline_prot32
ap_trampoline_prot32:
    mov   $0x10, %ax
    mov   %ax, %ds
    mov   %ax, %es
    mov   %ax, %ss
    mov   %cr4, %eax
    or    $0x20, %eax           # PAE
    mov   %eax, %cr4
    mov   TD_CR3(%ebx), %eax
    mov   %eax, %cr3
    mov   $0xc0000080, %ecx     # EFER
    rdmsr
    or    $0x100, %eax          # LME
    wrmsr
    mov   %cr0, %eax
    or    $0x80000000, %eax     # PG
    mov   %eax, %cr0
    ljmpl *TD_LONG64(%ebx)

.code64
.global ap_trampoline_long64
ap_trampoline_long64:
    # 32bit の mov で EBX に書いたので、RBX の上位 32 ビットは 0
    mov   TD_STACK(%rbx), %rsp
    mov   TD_ARG(%rbx), %rdi
    jmp   *TD_ENTRY(%rbx)
.global ap_trampoline_end
ap_trampoline_end:
"#,
    options(att_syntax)
);

/// AP に渡す情報。AP は初期化が終わったら `online` を立てる
struct ApStart {
    index: usize,
    tables: *mut gdt::CpuTables,
    online: AtomicBool,
}

extern "C" fn ap_entry(start: &'static ApStart) -> ! {
    gdt::load(unsafe { &mut *start.tables });
    idt::load();
    println!(
        "[smp] AP {} (APIC ID {}) online",
        start.index,
        current_apic_id()
    );
    start.online.store(true, Ordering::Release);

    if let Err(e) = vmx::enable() {
        panic!("AP {}: failed to enable VMX: {:?}", start.index, e);
    }
    // BSP が管理 VM の EPT を作るまで待ち、管理 VM の vCPU として SIPI を待つ。
    // 管理 VM の Linux が送った INIT / SIPI は VM exit になり、vCPU の状態に反映される
    let ept = *MANAGEMENT_VM_EPT.wait();
    let mut vcpu = Vcpu::new(&GuestConfig::wait_for_sipi(), ept);
    APS_IN_GUEST.fetch_add(1, Ordering::Release);
    let reason = vcpu.run();
    println!(
        "{} (AP {}): {:?}",
        crate::GUEST_STOPPED_MARKER,
        start.index,
        reason
    );
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// 管理 VM の EPT。BSP が管理 VM を起動するときに設定し、AP はそれを待ってから vCPU を作る
pub static MANAGEMENT_VM_EPT: Once<&'static Mutex<Ept>> = Once::new();
static APS_ONLINE: AtomicUsize = AtomicUsize::new(0);
static APS_IN_GUEST: AtomicUsize = AtomicUsize::new(0);

/// 起動したすべての AP が、管理 VM の vCPU として SIPI を待つ状態になるまで待つ。
/// 管理 VM の Linux が AP を起こす（INIT-SIPI-SIPI）前に呼ぶこと
pub fn wait_for_aps_in_guest() {
    let online = APS_ONLINE.load(Ordering::Acquire);
    while APS_IN_GUEST.load(Ordering::Acquire) < online {
        core::hint::spin_loop();
    }
}

/// Local APIC への IPI の送信
struct LocalApic {
    x2apic: bool,
    mmio: u64,
}

impl LocalApic {
    fn current() -> Self {
        const IA32_APIC_BASE: u32 = 0x1b;
        const X2APIC_ENABLE: u64 = 1 << 10;
        let base = cpu::rdmsr(IA32_APIC_BASE);
        LocalApic {
            x2apic: base & X2APIC_ENABLE != 0,
            mmio: base & 0x000f_ffff_ffff_f000,
        }
    }

    fn send_ipi(&self, apic_id: u32, icr_low: u32) {
        const X2APIC_ICR: u32 = 0x830;
        const ICR_LOW: u64 = 0x300;
        const ICR_HIGH: u64 = 0x310;
        const DELIVERY_PENDING: u32 = 1 << 12;
        if self.x2apic {
            unsafe { cpu::wrmsr(X2APIC_ICR, ((apic_id as u64) << 32) | icr_low as u64) };
            return;
        }
        // LAPIC の MMIO は HV の直接マップ（MTRR で UC）で見える
        let reg = |off: u64| (self.mmio + off) as *mut u32;
        unsafe {
            core::ptr::write_volatile(reg(ICR_HIGH), apic_id << 24);
            core::ptr::write_volatile(reg(ICR_LOW), icr_low);
            while core::ptr::read_volatile(reg(ICR_LOW)) & DELIVERY_PENDING != 0 {
                core::hint::spin_loop();
            }
        }
    }
}

/// ACPI PM タイマ（3.579545MHz）による待ち
struct PmTimer {
    port: u16,
    mask: u32,
}

impl PmTimer {
    const HZ: u64 = 3_579_545;

    fn read(&self) -> u32 {
        let v: u32;
        unsafe {
            asm!("in eax, dx", out("eax") v, in("dx") self.port, options(nomem, nostack, preserves_flags))
        };
        v & self.mask
    }

    /// 少なくとも `us` マイクロ秒待つ
    fn delay_us(&self, us: u64) {
        self.wait_until(us, || false);
    }

    /// `done` が true を返すか、`us` マイクロ秒たつまで待つ。`done` で終わったら true
    fn wait_until(&self, us: u64, mut done: impl FnMut() -> bool) -> bool {
        let ticks = us * Self::HZ / 1_000_000 + 1;
        let mut last = self.read();
        let mut elapsed = 0u64;
        while elapsed < ticks {
            if done() {
                return true;
            }
            let now = self.read();
            elapsed += (now.wrapping_sub(last) & self.mask) as u64;
            last = now;
            core::hint::spin_loop();
        }
        done()
    }
}

/// すべての AP を起動する。起動できた AP の数を返す
///
/// # Safety
/// `xsdt` のエントリが、読めるメモリ上の ACPI テーブルを指していること。
/// ゲストを起動する前（低位メモリのトランポリンを管理 VM が使っていないとき）に呼ぶこと
pub unsafe fn start_aps(
    topology: &CpuTopology,
    xsdt: &Xsdt,
    memory_map: &EfiMemoryMap,
    layout: &MemoryLayout,
) -> usize {
    let aps: Vec<u32> = topology
        .apic_ids
        .iter()
        .copied()
        .filter(|&id| id != topology.bsp_apic_id)
        .collect();
    if aps.is_empty() {
        return 0;
    }

    let (port, ext) = unsafe { xsdt.find::<Fadt>() }
        .and_then(|f| f.pm_timer())
        .expect("ACPI PM timer is required to start APs");
    let timer = PmTimer {
        port,
        mask: if ext { u32::MAX } else { 0x00ff_ffff },
    };

    // SIPI のベクタはページ番号なので、トランポリンは 1MiB 未満の 4KiB ページに置く
    let trampoline = mm::free_memory(memory_map, layout)
        .take(PAGE_SIZE_4K, PAGE_SIZE_4K, PAGE_SIZE_4K, 0x10_0000)
        .expect("no free page below 1MiB for the AP trampoline")
        .start;
    let cr3 = cpu::read_cr3();
    assert!(
        cr3 < 1 << 32,
        "the HV page table must be below 4GiB for the AP trampoline"
    );
    unsafe { write_trampoline(trampoline, cr3) };
    println!(
        "[smp] starting {} APs (trampoline {:#x})",
        aps.len(),
        trampoline
    );

    let lapic = LocalApic::current();
    const ICR_INIT: u32 = 0x4500; // INIT, level assert
    const ICR_STARTUP: u32 = 0x4600; // Start-up, level assert
    let vector = (trampoline >> 12) as u32;

    let mut online = 0;
    for (i, &apic_id) in aps.iter().enumerate() {
        let index = i + 1;
        let tables = unsafe {
            alloc::alloc::alloc_zeroed(Layout::new::<gdt::CpuTables>()) as *mut gdt::CpuTables
        };
        let stack =
            unsafe { alloc::alloc::alloc(Layout::from_size_align(AP_STACK_SIZE, 16).unwrap()) };
        assert!(
            !tables.is_null() && !stack.is_null(),
            "out of HV heap for AP {}",
            index
        );
        let start: &'static ApStart = Box::leak(Box::new(ApStart {
            index,
            tables,
            online: AtomicBool::new(false),
        }));

        let data = (trampoline as usize + TRAMPOLINE_DATA) as *mut u8;
        unsafe {
            // ap_entry に call で入ったときと同じアラインメント（RSP + 8 が 16 の倍数）にする
            write_u64(data, TD_STACK, stack as u64 + AP_STACK_SIZE as u64 - 8);
            write_u64(data, TD_ARG, start as *const ApStart as u64);
        }

        // INIT-SIPI-SIPI（SDM 10.4.4 / MultiProcessor Specification B.4）
        lapic.send_ipi(apic_id, ICR_INIT);
        timer.delay_us(10_000);
        for _ in 0..2 {
            lapic.send_ipi(apic_id, ICR_STARTUP | vector);
            timer.delay_us(200);
        }
        if timer.wait_until(AP_START_TIMEOUT_US, || start.online.load(Ordering::Acquire)) {
            online += 1;
            APS_ONLINE.store(online, Ordering::Release);
        } else {
            println!("[smp] AP {} (APIC ID {}) did not start", index, apic_id);
        }
    }
    online
}

unsafe fn write_u64(base: *mut u8, offset: usize, value: u64) {
    unsafe { core::ptr::write_unaligned(base.add(offset) as *mut u64, value) };
}

/// トランポリンのコードと、AP 共通のデータ（GDT、far pointer、CR3、エントリ）を書き込む
unsafe fn write_trampoline(page: u64, cr3: u64) {
    let start = (&raw const ap_trampoline_start) as usize;
    let offset_of = |sym: *const u8| sym as usize - start;
    let size = offset_of(&raw const ap_trampoline_end);
    assert!(size <= TRAMPOLINE_DATA, "the AP trampoline is too large");

    let base = page as *mut u8;
    let data = unsafe { base.add(TRAMPOLINE_DATA) };
    let far_ptr = |offset: usize, selector: u16| -> u64 {
        (page + offset as u64) | ((selector as u64) << 32)
    };
    unsafe {
        core::ptr::copy_nonoverlapping(start as *const u8, base, size);
        for (i, d) in TRAMPOLINE_GDT.iter().enumerate() {
            write_u64(data, TD_GDT + i * 8, *d);
        }
        let gdt_base = page + (TRAMPOLINE_DATA + TD_GDT) as u64;
        let gdt_limit = (TRAMPOLINE_GDT.len() * 8 - 1) as u64;
        write_u64(data, TD_GDTR, gdt_limit | (gdt_base << 16));
        write_u64(
            data,
            TD_PROT32,
            far_ptr(offset_of(&raw const ap_trampoline_prot32), TRAMPOLINE_CS32),
        );
        write_u64(
            data,
            TD_LONG64,
            far_ptr(offset_of(&raw const ap_trampoline_long64), TRAMPOLINE_CS64),
        );
        write_u64(data, TD_CR3, cr3);
        write_u64(data, TD_ENTRY, ap_entry as *const () as u64);
    }
}
