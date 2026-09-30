#![no_std]
#![no_main]

use core::arch::asm;

pub const COM1: usize = 0x3f8;

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
    return ret;
}

pub fn is_uart_available() -> bool {
    outb(0x3ff, 0xff);
    let val = inb(0x3ff);
    if val == 0xff {
        return true;
    } else {
        return false;
    }
}

#[inline(always)]
pub fn putc(c: u8) {
    outb(0x3f8, c);
}
