//! MxOS is an experimental OS developed by Max Shteimberg.
//! The goal is a highly asynchronious microkernel OS, with good security.
//!
//! NOTE: This should be updated if my goals change, or if my Engrish has improved.
//!

#![no_std]
#![feature(abi_x86_interrupt)]
// #![feature(asm)]
// #![feature(const_fn_trait_bound)]
#![feature(default_alloc_error_handler)]
#![warn(missing_docs)]

// extern crate alloc;

pub mod idt;
pub mod mem;
pub mod serial;

use core::panic::PanicInfo;
use mem::BumpAllocator;

// /// Internal stuff
// #[doc(hidden)]
// pub mod internals {
//     use super::*;
//
//     #[inline(always)]
//     #[no_mangle]
//     pub fn fmaxf(x: f32, y: f32) -> f32 {
//         libm::fmaxf(x, y)
//     }
//
//     #[inline(always)]
//     #[no_mangle]
//     pub fn fminf(x: f32, y: f32) -> f32 {
//         libm::fminf(x, y)
//     }
//
//     /// Truncates an f64 into an f32.
//     #[export_name = "__truncdfsf2"]
//     pub fn trunc_df2sf(x: f64) -> f32 {
//         let bits = x.to_bits();
//
//         let exp = (bits >> 52).checked_sub(1023 - 127).unwrap_or(0).min(255);
//         f32::from_bits((bits >> 32 & 1 << 31 | exp << 23 | bits >> 29 & (1 << 23) - 1) as _)
//     }
// }

fn init() {
    idt::init_idt();
    serial::init_logger();
}

/// The entry point of the kernel which starts everything.
#[no_mangle]
pub extern "C" fn kernel_main(multiboot_info_ptr: u32) -> ! {
    init();

    log::info!("Kernel main START");

    let boot_info = unsafe { multiboot2::load(multiboot_info_ptr as usize).unwrap() };
    log::info!("Loaded boot_info={:#?}", boot_info);
    let memory_map_tag = boot_info.memory_map_tag().expect("Memory Map tag required");
    let elf_sections_tag = boot_info
        .elf_sections_tag()
        .expect("ELF-Symbols tag required");

    log::info!("Memory areas: [");
    for area in memory_map_tag.memory_areas() {
        sprintln!(
            "    memory_area(addr=0x{:x}, size=0x{:x}),",
            area.start_address(),
            area.size(),
        );
    }
    sprintln!("]");

    log::info!("ELF sections: [");
    for section in elf_sections_tag.sections() {
        sprintln!(
            "    elf_section(addr=0x{:x}, size=0x{:x}, flags=0x{:x}),",
            section.start_address(),
            section.size(),
            section.flags(),
        )
    }
    sprintln!("]");

    let kernel_start = elf_sections_tag
        .sections()
        .map(|section| section.start_address())
        .min()
        .unwrap();
    let kernel_end = elf_sections_tag
        .sections()
        .map(|section| section.start_address())
        .max()
        .unwrap();

    let mut frame_allocator = BumpAllocator::new(
        [
            kernel_start..kernel_end,
            (boot_info.start_address() as _)..(boot_info.end_address() as _),
        ],
        memory_map_tag,
    );

    unsafe {
        mem::reset_page_table(kernel_start, kernel_end, &boot_info, &mut frame_allocator);
    }

    log::info!("Kernel main END");
    loop {}
}

/// The kernel panic handler.
#[panic_handler]
pub fn panic(info: &PanicInfo) -> ! {
    unsafe {
        serial::SERIAL_LOGGER.force_unlock();
    }

    log::error!("Kernel panic: `{}`", info);

    // log::error!("PANIC: {}", info);
    loop {}
}
