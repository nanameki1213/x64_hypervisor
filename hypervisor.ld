OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

KERNEL_OFFSET = 0xffffffff80000000;
KERNEL_PHYS   = 0x1000000;

SECTIONS
{
  . = KERNEL_OFFSET + KERNEL_PHYS;

  .text : AT(ADDR(.text) - KERNEL_OFFSET) {
    *(.text.boot) 
    *(.text .text.*)
  }

  .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET) {
    *(.rodata .rodata.*)
  }

  .eh_frame_hdr : AT(ADDR(.eh_frame_hdr) - KERNEL_OFFSET) { *(.eh_frame_hdr) }
  .eh_frame     : AT(ADDR(.eh_frame)     - KERNEL_OFFSET) { *(.eh_frame) }

  .hash     : AT(ADDR(.hash)     - KERNEL_OFFSET) { *(.hash) }
  .gnu.hash : AT(ADDR(.gnu.hash) - KERNEL_OFFSET) { *(.gnu.hash) }
  .dynsym   : AT(ADDR(.dynsym)   - KERNEL_OFFSET) { *(.dynsym) }
  .dynstr   : AT(ADDR(.dynstr)   - KERNEL_OFFSET) { *(.dynstr) }

  .rela.dyn : AT(ADDR(.rela.dyn) - KERNEL_OFFSET) {
    __rela_start = .;
    *(.rela.dyn)
    *(.rela.*)
    __rela_end = .;
  }

  . = ALIGN(4096);

  .dynamic : AT(ADDR(.dynamic) - KERNEL_OFFSET) { *(.dynamic) }

  .data : AT(ADDR(.data) - KERNEL_OFFSET) {
    *(.data.rel.ro .data.rel.ro.*)
    *(.data .data.*)
  }

  .bss : AT(ADDR(.bss) - KERNEL_OFFSET) {
    *(.bss .bss.*)
    *(COMMON)
  }

  /DISCARD/ : {
    *(.interp)
    *(.note .note.*)
    *(.comment)
  }
}
