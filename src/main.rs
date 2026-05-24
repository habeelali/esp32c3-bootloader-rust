#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::arch::global_asm;

mod rom;
mod soc;
mod init;
mod flash;
mod image;
mod partition;
mod utility;
mod sha256;
#[cfg(feature = "secure_boot")]
mod secure_boot_key;

/// Bootloader state tracking which partitions are available
#[derive(Default)]
pub struct BootloaderState {
    pub factory: PartitionPos,
    pub test: PartitionPos,
    pub ota: [PartitionPos; 16],
    pub ota_info: PartitionPos,
    pub app_count: u32,
}

#[derive(Clone, Copy, Default)]
pub struct PartitionPos {
    pub offset: u32,
    pub size: u32,
}

pub const TAG: &str = "boot";

global_asm!(
    r#"
    .section .init, "ax"
    .global _start
    .type _start, @function
    .option push
    .option norvc
_start:
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    la sp, _stack_start
    call rust_start
1:
    j 1b
    .option pop
"#
);

#[no_mangle]
extern "C" fn rust_start() -> ! {
    unsafe {
        // The ESP ROM image loader has already loaded .data into DRAM.
        let bss_start = &_bss_start as *const u32 as *mut u8;
        let bss_end = &_bss_end as *const u32 as *mut u8;
        let len = (bss_end as usize) - (bss_start as usize);
        core::ptr::write_bytes(bss_start, 0u8, len);
    }

    call_start_cpu0();
}

extern "C" {
    static _bss_start: u32;
    static _bss_end: u32;
}

fn call_start_cpu0() -> ! {
    // 1. Hardware initialization
    init::bootloader_init();

    // 2. Interactive shell (auto-boots after 5 s if no key pressed)
    match init::run_shell() {
        init::ShellAction::Reset => utility::bootloader_reset(),
        init::ShellAction::Boot => {}
    }

    // 3. Load partition table and select boot partition
    let mut bs = BootloaderState::default();
    let boot_index = utility::select_partition_number(&mut bs);

    if boot_index == utility::INVALID_INDEX {
        utility::bootloader_reset();
    }

    // 4. Load and boot the application image
    utility::bootloader_utility_load_boot_image(&bs, boot_index);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    init::debug_tx_str(b"[panic]");
    utility::bootloader_reset();
}
