//! HV 自身の例外ハンドラ。
//!
//! HV が例外を起こした時点で回復は諦め、原因の特定に必要な情報を出力して停止する。

use core::arch::{asm, global_asm};
use core::mem::size_of;

use console::println;

use super::gdt::{IST_DOUBLE_FAULT, IST_NMI, KERNEL_CS};

const NUM_EXCEPTIONS: usize = 32;

#[derive(Clone, Copy)]
#[repr(C)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            _reserved: 0,
        }
    }

    fn new(handler: u64, ist: u8) -> Self {
        IdtEntry {
            offset_low: handler as u16,
            selector: KERNEL_CS,
            ist,
            type_attr: 0x8e, // present, DPL0, 64bit interrupt gate
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

/// 例外発生時にスタックに積まれた状態。`exc_common` が積む順序と一致させること
#[repr(C)]
pub struct ExceptionFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    vector: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

// エラーコードを積まない例外はダミーの 0 を積み、全ベクタで同じフレーム形式にする
global_asm!(
    r#"
.macro EXC_STUB_NOERR n
exc_stub_\n:
    pushq $0
    pushq $\n
    jmp   exc_common
.endm

.macro EXC_STUB_ERR n
exc_stub_\n:
    pushq $\n
    jmp   exc_common
.endm

.section .text
.irp n, 0,1,2,3,4,5,6,7,9,15,16,18,19,20,22,23,24,25,26,27,28,31
    EXC_STUB_NOERR \n
.endr
.irp n, 8,10,11,12,13,14,17,21,29,30
    EXC_STUB_ERR \n
.endr

exc_common:
    pushq %rax
    pushq %rbx
    pushq %rcx
    pushq %rdx
    pushq %rsi
    pushq %rdi
    pushq %rbp
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    mov   %rsp, %rdi
    call  exception_handler
    popq  %r15
    popq  %r14
    popq  %r13
    popq  %r12
    popq  %r11
    popq  %r10
    popq  %r9
    popq  %r8
    popq  %rbp
    popq  %rdi
    popq  %rsi
    popq  %rdx
    popq  %rcx
    popq  %rbx
    popq  %rax
    add   $16, %rsp # vector, error_code
    iretq

.section .rodata
.balign 8
.global exc_stub_table
exc_stub_table:
.irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad exc_stub_\n
.endr
"#,
    options(att_syntax)
);

unsafe extern "C" {
    static exc_stub_table: [u64; NUM_EXCEPTIONS];
}

const EXCEPTION_NAMES: [&str; NUM_EXCEPTIONS] = [
    "#DE Divide Error",
    "#DB Debug",
    "NMI",
    "#BP Breakpoint",
    "#OF Overflow",
    "#BR BOUND Range Exceeded",
    "#UD Invalid Opcode",
    "#NM Device Not Available",
    "#DF Double Fault",
    "Coprocessor Segment Overrun",
    "#TS Invalid TSS",
    "#NP Segment Not Present",
    "#SS Stack-Segment Fault",
    "#GP General Protection",
    "#PF Page Fault",
    "Reserved",
    "#MF x87 FPU Error",
    "#AC Alignment Check",
    "#MC Machine Check",
    "#XM SIMD Floating-Point",
    "#VE Virtualization",
    "#CP Control Protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "#HV Hypervisor Injection",
    "#VC VMM Communication",
    "#SX Security",
    "Reserved",
];

pub fn init() {
    unsafe {
        let idt = &raw mut IDT;
        for (vector, &stub) in exc_stub_table.iter().enumerate() {
            let ist = match vector {
                2 => IST_NMI,
                8 => IST_DOUBLE_FAULT,
                _ => 0,
            };
            (*idt)[vector] = IdtEntry::new(stub, ist);
        }

        let ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: idt as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}

#[unsafe(no_mangle)]
extern "C" fn exception_handler(frame: &ExceptionFrame) {
    // 例外がコンソールのロック中に起きた可能性があるため、強制的に解放する
    unsafe { console::CONSOLE.force_unlock() };

    let (cr0, cr2, cr3, cr4): (u64, u64, u64, u64);
    unsafe {
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
    }

    let name = EXCEPTION_NAMES
        .get(frame.vector as usize)
        .copied()
        .unwrap_or("Unknown");
    println!(
        "[EXCEPTION] vector={} ({}) error_code={:#x}",
        frame.vector, name, frame.error_code
    );
    println!(
        "  RIP={:#018x} CS={:#06x} RFLAGS={:#018x}",
        frame.rip, frame.cs, frame.rflags
    );
    println!("  RSP={:#018x} SS={:#06x}", frame.rsp, frame.ss);
    println!(
        "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
        frame.rax, frame.rbx, frame.rcx, frame.rdx
    );
    println!(
        "  RSI={:#018x} RDI={:#018x} RBP={:#018x}",
        frame.rsi, frame.rdi, frame.rbp
    );
    println!(
        "  R8 ={:#018x} R9 ={:#018x} R10={:#018x} R11={:#018x}",
        frame.r8, frame.r9, frame.r10, frame.r11
    );
    println!(
        "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
        frame.r12, frame.r13, frame.r14, frame.r15
    );
    println!(
        "  CR0={:#018x} CR2={:#018x} CR3={:#018x} CR4={:#018x}",
        cr0, cr2, cr3, cr4
    );

    crate::halt_loop();
}
