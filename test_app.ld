SECTIONS
{
    . = 0x40380000;
    .text : {
        *(.entry.text)
        *(.text*)
        *(.rodata*)
        *(.srodata*)
    }
    . = ALIGN(4);
    _image_end = .;
}
