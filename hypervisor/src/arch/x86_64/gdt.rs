use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const TSS_SEL: u16 = 0x18;

/// #DF / NMI 用の IST 番号（IDT エントリに設定する値）
pub const IST_DOUBLE_FAULT: u8 = 1;
pub const IST_NMI: u8 = 2;

const IST_STACK_SIZE: usize = 0x4000;

#[repr(C, packed)]
struct Tss {
    _reserved0: u32,
    rsp: [u64; 3],
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

// null, kernel code, kernel data, TSS (16 バイト = 2 エントリ。load で設定する)
const GDT_TEMPLATE: [u64; 5] = [
    0,
    0x00af_9a00_0000_ffff, // 64bit code, DPL0
    0x00cf_9200_0000_ffff, // data, DPL0
    0,
    0,
];

/// CPU ごとに必要な GDT / TSS / IST スタック。
/// TSS ディスクリプタの busy ビットは CPU ごとに立つので、GDT も CPU ごとに持つ
#[repr(C)]
pub struct CpuTables {
    gdt: [u64; 5],
    tss: Tss,
    double_fault_stack: IstStack,
    nmi_stack: IstStack,
}

impl CpuTables {
    pub const fn new() -> Self {
        CpuTables {
            gdt: GDT_TEMPLATE,
            tss: Tss {
                _reserved0: 0,
                rsp: [0; 3],
                _reserved1: 0,
                ist: [0; 7],
                _reserved2: 0,
                _reserved3: 0,
                iomap_base: size_of::<Tss>() as u16,
            },
            double_fault_stack: IstStack([0; IST_STACK_SIZE]),
            nmi_stack: IstStack([0; IST_STACK_SIZE]),
        }
    }
}

/// BSP 用。ヒープができる前に使うので静的に置く（AP 用はヒープから確保する）
static mut BSP_TABLES: CpuTables = CpuTables::new();

/// BSP の GDT / TSS を設定する
pub fn init() {
    // init は BSP で一度だけ呼ぶので、BSP_TABLES への参照はほかにない
    load(unsafe { (&raw mut BSP_TABLES).as_mut() }.unwrap());
}

/// 現在の CPU に `tables` の GDT / TSS を読み込む。`tables` はこの CPU 専用で、以後解放しないこと。
/// ディスクリプタはここで設定するので、`tables` はゼロ初期化したものでもよい
pub fn load(tables: &'static mut CpuTables) {
    // CpuTables::new() はスタック上に IST スタックごと作ってしまうので、必要な値だけを書く
    tables.gdt = GDT_TEMPLATE;
    tables.tss.iomap_base = size_of::<Tss>() as u16;
    unsafe {
        let tss = &raw mut tables.tss;
        (*tss).ist[(IST_DOUBLE_FAULT - 1) as usize] =
            (&raw const tables.double_fault_stack) as u64 + IST_STACK_SIZE as u64;
        (*tss).ist[(IST_NMI - 1) as usize] =
            (&raw const tables.nmi_stack) as u64 + IST_STACK_SIZE as u64;

        let base = tss as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        let gdt = &raw mut tables.gdt;
        // 64bit TSS descriptor (type = 0x9: available TSS, present)
        (*gdt)[3] = (limit & 0xffff)
            | ((base & 0xff_ffff) << 16)
            | (0x89 << 40)
            | (((limit >> 16) & 0xf) << 48)
            | (((base >> 24) & 0xff) << 56);
        (*gdt)[4] = base >> 32;

        let ptr = DescriptorTablePointer {
            limit: (size_of::<[u64; 5]>() - 1) as u16,
            base: gdt as u64,
        };
        asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));

        // CS を far return で再ロードし、データセグメントを設定する
        asm!(
            "push {cs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {ds:x}",
            "mov es, {ds:x}",
            "mov ss, {ds:x}",
            "xor {tmp:e}, {tmp:e}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            cs = in(reg) KERNEL_CS as u64,
            ds = in(reg) KERNEL_DS as u64,
            tmp = out(reg) _,
        );

        asm!("ltr {:x}", in(reg) TSS_SEL, options(nostack, preserves_flags));
    }
    debug_assert_eq!(tss_base(), (&raw const tables.tss) as u64);
}

/// 現在の CPU の TSS のアドレス（VMCS の TR base に設定する）。
/// GDT の TSS ディスクリプタから読むので、どの CPU で呼んでもその CPU の TSS を返す
pub fn tss_base() -> u64 {
    let gdtr = super::cpu::sgdt();
    let index = (super::cpu::read_tr() >> 3) as u64;
    let (low, high) = unsafe {
        let entries = gdtr.base as *const u64;
        (
            *entries.add(index as usize),
            *entries.add(index as usize + 1),
        )
    };
    ((low >> 16) & 0xff_ffff) | (((low >> 56) & 0xff) << 24) | (high << 32)
}

pub fn tss_limit() -> u64 {
    (size_of::<Tss>() - 1) as u64
}
