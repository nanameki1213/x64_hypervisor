pub mod gdt;
pub mod idt;

/// HV 自身の GDT / TSS / IDT を設定する。
///
/// UEFI が用意した GDT / IDT は BootServicesData 上にあり、ヒープとして再利用すると
/// 壊れるため、メモリマップをヒープに登録する前に呼ぶこと。
pub fn init() {
    gdt::init();
    idt::init();
}
