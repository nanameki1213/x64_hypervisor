OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(main)

MEMORY
{
  VIRT (rxw) : ORIGIN = 0xFFFFFFFF80000000, LENGTH = 0x40000000
  PHYS (rxw) : ORIGIN = 0x00100000,         LENGTH = 0x40000000
}

SECTIONS
{
  .hash         : { *(.hash)         } > VIRT AT> PHYS
  .gnu.hash     : { *(.gnu.hash)     } > VIRT AT> PHYS
  .dynsym       : { *(.dynsym)       } > VIRT AT> PHYS
  .dynstr       : { *(.dynstr)       } > VIRT AT> PHYS
  .eh_frame_hdr : { *(.eh_frame_hdr) } > VIRT AT> PHYS
  .eh_frame     : { *(.eh_frame)     } > VIRT AT> PHYS

  .text : {
    *(.text)
    *(.text.*)
  } > VIRT AT> PHYS

  .rodata : {
    *(.rodata .rodata.*)
  } > VIRT AT> PHYS

  .data : {
    *(.data .data.*)
    *(.dynamic)
  } > VIRT AT> PHYS

  .bss : {
    *(.bss)
    *(COMMON)
  } > VIRT AT> PHYS
}
