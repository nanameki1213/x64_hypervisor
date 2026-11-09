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

// null, kernel code, kernel data, TSS (16 バイト = 2 エントリ)
static mut GDT: [u64; 5] = [
    0,
    0x00af_9a00_0000_ffff, // 64bit code, DPL0
    0x00cf_9200_0000_ffff, // data, DPL0
    0,
    0,
];

static mut TSS: Tss = Tss {
    _reserved0: 0,
    rsp: [0; 3],
    _reserved1: 0,
    ist: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: size_of::<Tss>() as u16,
};

static mut DOUBLE_FAULT_STACK: IstStack = IstStack([0; IST_STACK_SIZE]);
static mut NMI_STACK: IstStack = IstStack([0; IST_STACK_SIZE]);

pub fn init() {
    unsafe {
        let tss = &raw mut TSS;
        (*tss).ist[(IST_DOUBLE_FAULT - 1) as usize] =
            (&raw const DOUBLE_FAULT_STACK) as u64 + IST_STACK_SIZE as u64;
        (*tss).ist[(IST_NMI - 1) as usize] = (&raw const NMI_STACK) as u64 + IST_STACK_SIZE as u64;

        let base = tss as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        let gdt = &raw mut GDT;
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
}

/// VMCS の TR base に設定する TSS のアドレス
pub fn tss_base() -> u64 {
    (&raw const TSS) as u64
}

pub fn tss_limit() -> u64 {
    (size_of::<Tss>() - 1) as u64
}
