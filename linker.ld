/* ESP32-C3 2nd stage bootloader linker script */

MEMORY
{
    /* ESP32-C3 reserves the low SRAM/IRAM window for cache and ROM use.
       Place the second-stage bootloader in the high bootloader IRAM window. */
    IRAM : ORIGIN = 0x403CE000, LENGTH = 0x8000
    DRAM : ORIGIN = 0x3FC80000, LENGTH = 0x8000
}

REGION_ALIAS("REGION_TEXT", IRAM);
REGION_ALIAS("REGION_RODATA", IRAM);
REGION_ALIAS("REGION_DATA", DRAM);
REGION_ALIAS("REGION_BSS", DRAM);

ENTRY(_start)

SECTIONS
{
    .text : ALIGN(4)
    {
        _stext = .;
        *(.init);
        *(.init.*);
        *(.text);
        *(.text.*);
        *(.rodata);
        *(.rodata.*);
        _etext = .;
    } > REGION_TEXT

    /* .data follows .text in flash, loaded by ROM, copied to DRAM at startup */
    .data : ALIGN(4)
    {
        _data_start = .;
        PROVIDE(__global_pointer$ = _data_start + 0x800);
        *(.data);
        *(.data.*);
        *(.sdata);
        *(.sdata.*);
        _data_end = .;
    } > REGION_DATA AT > REGION_TEXT

    /* .bss is only in DRAM, zeroed at startup */
    .bss (NOLOAD) : ALIGN(4)
    {
        _bss_start = .;
        *(.bss);
        *(.bss.*);
        *(.sbss);
        *(.sbss.*);
        *(COMMON);
        _bss_end = .;
    } > REGION_DATA

    _stack_start = ORIGIN(DRAM) + LENGTH(DRAM);
    _heap_start = _bss_end;
    _heap_end = _stack_start;

    /* Symbols for data copy at startup */
    _data_load_start = LOADADDR(.data);
    _data_load_end = _data_load_start + SIZEOF(.data);
}
