use std::{path::Path, time::Instant};

use probe_rs::{
    config::MemoryRegion,
    flashing::{download_file_with_options, DownloadOptions, FlashProgress, Format},
    Architecture, Core, MemoryInterface, Session,
};

use anyhow::{Context, Result};

pub fn test_register_access(core: &mut Core) -> Result<()> {
    println!("Testing register access...");

    let register = core.registers();

    let mut test_value = 1;

    for register in register.registers() {
        // Skip register x0 on RISCV chips, it's hardwired to zero.
        if core.architecture() == Architecture::Riscv && register.name() == "x0" {
            continue;
        }

        // Write new value

        core.write_core_reg(register.into(), test_value)?;

        let readback = core.read_core_reg(register)?;

        assert_eq!(
            test_value, readback,
            "Error writing register {:?}, read value does not match written value.",
            register
        );

        test_value = test_value.wrapping_shl(1);
    }

    Ok(())
}

pub fn test_memory_access(core: &mut Core, memory_regions: &[MemoryRegion]) -> Result<()> {
    // Try to write all memory regions
    for region in memory_regions {
        match region {
            probe_rs::config::MemoryRegion::Ram(ram) => {
                let ram_start = ram.range.start;
                let ram_size = ram.range.end - ram.range.start;

                println!("Test - RAM Start 32");
                // Write first word
                core.write_word_32(ram_start, 0xababab)?;
                let value = core.read_word_32(ram_start)?;
                assert!(value == 0xababab);

                println!("Test - RAM End 32");
                // Write last word
                core.write_word_32(ram_start + ram_size - 4, 0xababac)?;
                let value = core.read_word_32(ram_start + ram_size - 4)?;
                assert!(value == 0xababac);

                println!("Test - RAM Start 8");
                // Write first byte
                core.write_word_8(ram_start, 0xac)?;
                let value = core.read_word_8(ram_start)?;
                assert!(value == 0xac);

                println!("Test - RAM 8 Unaligned");
                let address = ram_start + 1;
                let data = 0x23;
                // Write last byte
                core.write_word_8(address, data)
                    .with_context(|| format!("Write_word_8 to address {:08x}", address))?;

                let value = core
                    .read_word_8(address)
                    .with_context(|| format!("read_word_8 from address {:08x}", address))?;
                assert!(value == data);

                println!("Test - RAM End 8");
                // Write last byte
                core.write_word_8(ram_start + ram_size - 1, 0xcd)
                    .with_context(|| {
                        format!("Write_word_8 to address {:08x}", ram_start + ram_size - 1)
                    })?;

                let value = core
                    .read_word_8(ram_start + ram_size - 1)
                    .with_context(|| {
                        format!("read_word_8 from address {:08x}", ram_start + ram_size - 1)
                    })?;
                assert!(value == 0xcd);
            }
            // Ignore other types of regions
            _other => {}
        }
    }

    Ok(())
}

pub fn test_hw_breakpoints(core: &mut Core, memory_regions: &[MemoryRegion]) -> Result<()> {
    println!("Testing HW breakpoints");

    // For this test, we assume that code is executed from Flash / non-volatile memory, and try to set breakpoints
    // in these regions.
    for region in memory_regions {
        match region {
            probe_rs::config::MemoryRegion::Nvm(nvm) => {
                let initial_breakpoint_addr = nvm.range.start;

                let num_breakpoints = core.get_available_breakpoint_units()?;

                println!("{} breakpoints supported", num_breakpoints);

                for i in 0..num_breakpoints {
                    core.set_hw_breakpoint(initial_breakpoint_addr + 4 * i)?;
                }

                // Try to set an additional breakpoint, which should fail
                core.set_hw_breakpoint(initial_breakpoint_addr + num_breakpoints * 4)
                    .expect_err(
                        "Trying to use more than supported number of breakpoints should fail.",
                    );

                // Clear all breakpoints again
                for i in 0..num_breakpoints {
                    core.clear_hw_breakpoint(initial_breakpoint_addr + 4 * i)?;
                }
            }

            // Skip other regions
            _other => {}
        }
    }

    Ok(())
}

pub fn test_flashing(session: &mut Session, test_binary: &Path) -> Result<()> {
    let progress = FlashProgress::new(|event| {
        log::debug!("Flash Event: {:?}", event);
        eprint!(".");
    });

    let options = DownloadOptions {
        progress: Some(&progress),
        ..Default::default()
    };

    println!("Starting flashing test");
    println!("Binary: {}", test_binary.display());

    let start_time = Instant::now();

    download_file_with_options(session, test_binary, Format::Elf, options)?;

    println!();

    println!("Total time for flashing: {:.2?}", start_time.elapsed());

    println!("Finished flashing");

    Ok(())
}
