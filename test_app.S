# ESP32-C3 shell test app entry.

.section .entry.text, "ax"
.globl _entry
_entry:
    # Give the C shell a known-good stack near the top of DRAM.
    li      sp, 0x3fcdf000
    tail    app_main
