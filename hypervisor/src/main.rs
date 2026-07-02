#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[unsafe(no_mangle)]
extern "win64" fn main() {
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
