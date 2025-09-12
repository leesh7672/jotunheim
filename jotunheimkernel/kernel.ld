/* kernel.ld — x86_64 bare-metal EXEC kernel */
OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

/* Optional: make segment alignment explicit */
MAXPAGESIZE = 0x500000;   /* 500 MiB */

PHDRS {
  text   PT_LOAD FLAGS(5); /* R + X */
  rodata PT_LOAD FLAGS(4); /* R     */
  data   PT_LOAD FLAGS(6); /* R + W */
}

SECTIONS
{
  /* Place image at/after 1 MiB so it’s in the identity-mapped low range. */
  . = 1M;

  /* ---- Text (code) ---- */
  .text : ALIGN(4K)
  {
    /* ensure _start is the very first byte in .text */
    KEEP(*(.text._start))
    *(.text .text.*)
  } :text

  /* ---- Read-only data ---- */
  .rodata : ALIGN(4K)
  {
    *(.rodata .rodata.*)
  } :rodata

  /* ---- Data ---- */
  .data : ALIGN(4K)
  {
    *(.data .data.*)
    *(.sdata .sdata.*)
  } :data

  /* ---- BSS ---- */
  .bss (NOLOAD) : ALIGN(4K)
  {
    __bss_start = .;
    *(.bss .bss.*)
    *(COMMON)
    __bss_end = .;
  } :data

  /* Useful end markers */
  __kernel_start = ADDR(.text);
  __kernel_end   = .;
  __text_start   = ADDR(.text);
  __text_end     = ADDR(.text)   + SIZEOF(.text);
  __rodata_start = ADDR(.rodata);
  __rodata_end   = ADDR(.rodata) + SIZEOF(.rodata);
  __data_start   = ADDR(.data);
  __data_end     = ADDR(.data)   + SIZEOF(.data);
  __bss_start    = ADDR(.bss);
  __bss_end      = ADDR(.bss)    + SIZEOF(.bss);

  /* Discard sections we don’t need */
  /DISCARD/ :
  {
    *(.eh_frame*)
    *(.comment*)
    *(.note*)
    *(.gnu.hash*)
  }
}
