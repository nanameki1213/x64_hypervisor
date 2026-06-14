#![no_std]

use core::fmt;
use spin::Mutex;

/// 汎用コンソール。
///
/// 出力バックエンドを `fn(u8)` で注入するため、UEFI・シリアル・VGA など
/// 特定の出力先に依存しない。
///
/// # 使い方
/// 1. 出力先に合わせた `write_byte: fn(u8)` を用意する。
/// 2. `*CONSOLE.lock() = Some(Console::new(write_byte));` で初期化する。
/// 3. `print!` / `println!` マクロで書き込む。
pub struct Console {
    write_byte: fn(u8),
}

impl Console {
    pub const fn new(write_byte: fn(u8)) -> Self {
        Console { write_byte }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            (self.write_byte)(b);
        }
        Ok(())
    }
}

pub static CONSOLE: Mutex<Option<Console>> = Mutex::new(None);

pub fn print(args: fmt::Arguments) {
    use fmt::Write;
    if let Some(console) = CONSOLE.lock().as_mut() {
        let _ = console.write_fmt(args);
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    ()            => ($crate::print!("\n"));
    ($($arg:tt)+) => ($crate::print(format_args!("{}\n", format_args!($($arg)+))));
}
