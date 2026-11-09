#![no_std]
#![no_main]

use core::arch::asm;

/// 管理 VM（Linux）のコンソール用。HV は使わない
pub const COM1: u16 = 0x3f8;
/// HV のログ出力用
pub const COM2: u16 = 0x2f8;

// 16550 UART のレジスタオフセット
const REG_DATA: u16 = 0;
const REG_INT_ENABLE: u16 = 1;
const REG_FIFO_CTRL: u16 = 2;
const REG_LINE_CTRL: u16 = 3;
const REG_MODEM_CTRL: u16 = 4;
const REG_LINE_STATUS: u16 = 5;
const REG_SCRATCH: u16 = 7;

const LSR_THR_EMPTY: u8 = 1 << 5;

#[inline(always)]
pub fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val);
    }
}

#[inline(always)]
pub fn inb(port: u16) -> u8 {
    let mut ret: u8 = 0;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") ret, options(nomem, nostack, preserves_flags));
    }
    ret
}

/// scratch レジスタへの書き込みが読み返せるかで UART の有無を判定する
pub fn is_uart_available(port: u16) -> bool {
    outb(port + REG_SCRATCH, 0xa5);
    inb(port + REG_SCRATCH) == 0xa5
}

/// 115200 baud, 8N1, FIFO 有効, 割り込み無効で初期化する
pub fn init(port: u16) {
    outb(port + REG_INT_ENABLE, 0x00);
    outb(port + REG_LINE_CTRL, 0x80); // DLAB = 1
    outb(port + REG_DATA, 0x01); // divisor low  (115200 / 1)
    outb(port + REG_INT_ENABLE, 0x00); // divisor high
    outb(port + REG_LINE_CTRL, 0x03); // DLAB = 0, 8N1
    outb(port + REG_FIFO_CTRL, 0xc7); // FIFO 有効・クリア
    outb(port + REG_MODEM_CTRL, 0x03); // DTR | RTS
}

#[inline(always)]
pub fn write_byte(port: u16, c: u8) {
    while inb(port + REG_LINE_STATUS) & LSR_THR_EMPTY == 0 {
        core::hint::spin_loop();
    }
    outb(port + REG_DATA, c);
}
