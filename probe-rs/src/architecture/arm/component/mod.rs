//! Types and functions for interacting with CoreSight Components

mod dwt;
mod itm;
mod swo;
mod tmc;
mod tpiu;
mod trace_funnel;

use super::memory::romtable::{CoresightComponent, PeripheralType, RomTableError};
use crate::architecture::arm::core::armv6m::Demcr;
use crate::architecture::arm::{ArmProbeInterface, SwoConfig, SwoMode};
use crate::{Core, Error, MemoryInterface, MemoryMappedRegister};

pub use self::itm::Itm;
pub use dwt::Dwt;
pub use swo::Swo;
pub use tmc::TraceMemoryController;
pub use tpiu::Tpiu;
pub use trace_funnel::TraceFunnel;

/// Specifies the data sink (destination) for trace data.
#[derive(Debug, Copy, Clone)]
pub enum TraceSink {
    /// Trace data should be sent to the SWO peripheral.
    ///
    /// # Note
    /// On some architectures, there is no distinction between SWO and TPIU.
    Swo(SwoConfig),

    /// Trace data should be sent to the TPIU peripheral.
    Tpiu(SwoConfig),

    /// Trace data should be sent to the embedded trace buffer for software-based trace collection.
    TraceMemory,
}

/// An error when operating a core ROM table component occurred.
#[derive(thiserror::Error, Debug)]
pub enum ComponentError {
    /// Nordic chips do not support setting all TPIU clocks. Try choosing another clock speed.
    #[error("Nordic does not support TPIU CLK value of {0}")]
    NordicUnsupportedTPUICLKValue(u32),
}

/// A trait to be implemented on debug register types for debug component interfaces.
pub trait DebugRegister: Clone + From<u32> + Into<u32> + Sized + std::fmt::Debug {
    /// The address of the register.
    const ADDRESS: u32;
    /// The name of the register.
    const NAME: &'static str;

    /// Loads the register value from the given debug component via the given core.
    fn load(
        component: &CoresightComponent,
        interface: &mut dyn ArmProbeInterface,
    ) -> Result<Self, Error> {
        Ok(Self::from(component.read_reg(interface, Self::ADDRESS)?))
    }

    /// Loads the register value from the given component in given unit via the given core.
    fn load_unit(
        component: &CoresightComponent,
        interface: &mut dyn ArmProbeInterface,
        unit: usize,
    ) -> Result<Self, Error> {
        Ok(Self::from(
            component.read_reg(interface, Self::ADDRESS + 16 * unit as u32)?,
        ))
    }

    /// Stores the register value to the given debug component via the given core.
    fn store(
        &self,
        component: &CoresightComponent,
        interface: &mut dyn ArmProbeInterface,
    ) -> Result<(), Error> {
        component.write_reg(interface, Self::ADDRESS, self.clone().into())
    }

    /// Stores the register value to the given component in given unit via the given core.
    fn store_unit(
        &self,
        component: &CoresightComponent,
        interface: &mut dyn ArmProbeInterface,
        unit: usize,
    ) -> Result<(), Error> {
        component.write_reg(
            interface,
            Self::ADDRESS + 16 * unit as u32,
            self.clone().into(),
        )
    }
}

/// Goes through every component in the vector and tries to find the first component with the given type
fn find_component(
    components: &[CoresightComponent],
    peripheral_type: PeripheralType,
) -> Result<&CoresightComponent, Error> {
    components
        .iter()
        .find_map(|component| component.find_component(peripheral_type))
        .ok_or_else(|| {
            Error::architecture_specific(RomTableError::ComponentNotFound(peripheral_type))
        })
}

/// Configure the Trace Port Interface Unit
///
/// # Note
/// This configures the TPIU in serial wire mode.
///
/// # Args
/// * `interface` - The interface with the probe.
/// * `component` - The TPIU CoreSight component found.
/// * `config` - The SWO pin configuration to use.
fn configure_tpiu(
    interface: &mut dyn ArmProbeInterface,
    component: &CoresightComponent,
    config: &SwoConfig,
) -> Result<(), Error> {
    let mut tpiu = Tpiu::new(interface, component);

    tpiu.set_port_size(1)?;
    let prescaler = (config.tpiu_clk() / config.baud()) - 1;
    tpiu.set_prescaler(prescaler)?;
    match config.mode() {
        SwoMode::Manchester => tpiu.set_pin_protocol(1)?,
        SwoMode::Uart => tpiu.set_pin_protocol(2)?,
    }

    // Formatter: TrigIn enabled, bypass optional
    if config.tpiu_continuous_formatting() {
        // Set EnFCont for continuous formatting even over SWO.
        tpiu.set_formatter(0x102)?;
    } else {
        // Clear EnFCont to only pass through raw ITM/DWT data.
        tpiu.set_formatter(0x100)?;
    }

    Ok(())
}

/// Sets up all the SWV components.
///
/// Expects to be given a list of all ROM table `components` as the second argument.
pub(crate) fn setup_tracing(
    interface: &mut dyn ArmProbeInterface,
    components: &[CoresightComponent],
    sink: &TraceSink,
) -> Result<(), Error> {
    // Configure DWT
    let mut dwt = Dwt::new(interface, find_component(components, PeripheralType::Dwt)?);
    dwt.enable()?;
    dwt.enable_exception_trace()?;

    // Configure ITM
    let mut itm = Itm::new(interface, find_component(components, PeripheralType::Itm)?);
    itm.unlock()?;
    itm.tx_enable()?;

    // Configure the trace destination.
    match sink {
        TraceSink::Tpiu(config) => {
            configure_tpiu(
                interface,
                find_component(components, PeripheralType::Tpiu)?,
                config,
            )?;
        }

        TraceSink::Swo(config) => {
            if let Ok(peripheral) = find_component(components, PeripheralType::Swo) {
                let mut swo = Swo::new(interface, peripheral);
                swo.unlock()?;

                let prescaler = (config.tpiu_clk() / config.baud()) - 1;
                swo.set_prescaler(prescaler)?;

                match config.mode() {
                    SwoMode::Manchester => swo.set_pin_protocol(1)?,
                    SwoMode::Uart => swo.set_pin_protocol(2)?,
                }
            } else {
                // For Cortex-M4, the SWO and the TPIU are combined. If we don't find a SWO
                // peripheral, use the TPIU instead.
                configure_tpiu(
                    interface,
                    find_component(components, PeripheralType::Tpiu)?,
                    config,
                )?;
            }
        }

        TraceSink::TraceMemory => {
            let mut tmc = TraceMemoryController::new(
                interface,
                find_component(components, PeripheralType::Tmc)?,
            );

            // Clear out the TMC FIFO before initiating the capture.
            tmc.disable_capture()?;
            while !tmc.ready()? {}

            // Configure the TMC for software-polled mode, as we will read out data using the debug
            // interface.
            tmc.set_mode(tmc::Mode::Software)?;

            tmc.enable_capture()?;
        }
    }

    Ok(())
}

/// Read trace data from internal trace memory
///
/// # Args
/// * `interface` - The interface with the debug probe.
/// * `components` - The CoreSight debug components identified in the system.
///
/// # Note
/// This function will read any available trace data in trace memory without blocking. At most,
/// this function will read as much data as can fit in the FIFO - if the FIFO continues to be
/// filled while trace data is being extracted, this function can be called again to return that
/// data.
///
/// # Returns
/// All data stored in trace memory, with an upper bound at the size of internal trace memory.
pub(crate) fn read_trace_memory(
    interface: &mut dyn ArmProbeInterface,
    components: &[CoresightComponent],
) -> Result<Vec<u8>, Error> {
    let mut tmc =
        TraceMemoryController::new(interface, find_component(components, PeripheralType::Tmc)?);

    let fifo_size = tmc.fifo_size()?;

    // This sequence is taken from "CoreSight Trace memory Controller Technical Reference Manual"
    // Section 2.2.2 "Software FIFO Mode". Without following this procedure, the trace data does
    // not properly stop even after disabling capture.

    // Read all of the data from the ETM into a vector for further processing.
    let mut etf_trace: Vec<u8> = Vec::new();
    loop {
        match tmc.read()? {
            Some(data) => etf_trace.extend_from_slice(&data.to_le_bytes()),
            None => {
                // If there's nothing available in the FIFO, we can only break out of reading if we
                // have an integer number of formatted frames, which are 16 bytes each.
                if (etf_trace.len() % 16) == 0 {
                    break;
                }
            }
        }

        // If the FIFO is being filled faster than we can read it, break out after reading a
        // maximum number of frames.
        let frame_boundary = (etf_trace.len() % 16) == 0;

        if frame_boundary && etf_trace.len() >= fifo_size as usize {
            break;
        }
    }

    // The TMC formats data into frames, as it contains trace data from multiple data sources. We
    // need to deserialize the frames and pull out only the data source of interest. For now, all
    // we care about is the ITM data.

    let mut id = 0.into();
    let mut itm_trace = Vec::new();

    // Process each formatted frame and extract the multiplexed trace data.
    for frame_buffer in etf_trace.chunks_exact(16) {
        let mut frame = tmc::Frame::new(frame_buffer, id);
        for (id, data) in &mut frame {
            match id.into() {
                // ITM ATID, see Itm::tx_enable()
                13 => itm_trace.push(data),
                0 => (),
                id => tracing::warn!("Unexpected trace source ATID {id}: {data}, ignoring"),
            }
        }
        id = frame.id();
    }

    Ok(itm_trace)
}

/// Configures DWT trace unit `unit` to begin tracing `address`.
///
///
/// Expects to be given a list of all ROM table `components` as the second argument.
pub(crate) fn add_swv_data_trace(
    interface: &mut dyn ArmProbeInterface,
    components: &[CoresightComponent],
    unit: usize,
    address: u32,
) -> Result<(), Error> {
    let mut dwt = Dwt::new(interface, find_component(components, PeripheralType::Dwt)?);
    dwt.enable_data_trace(unit, address)
}

/// Configures DWT trace unit `unit` to stop tracing `address`.
///
///
/// Expects to be given a list of all ROM table `components` as the second argument.
pub fn remove_swv_data_trace(
    interface: &mut dyn ArmProbeInterface,
    components: &[CoresightComponent],
    unit: usize,
) -> Result<(), Error> {
    let mut dwt = Dwt::new(interface, find_component(components, PeripheralType::Dwt)?);
    dwt.disable_data_trace(unit)
}

/// Sets TRCENA in DEMCR to begin trace generation.
pub fn enable_tracing(core: &mut Core) -> Result<(), Error> {
    let mut demcr = Demcr(core.read_word_32(Demcr::ADDRESS)?);
    demcr.set_dwtena(true);
    core.write_word_32(Demcr::ADDRESS, demcr.into())?;
    Ok(())
}

/// Disables TRCENA in DEMCR to disable trace generation.
pub fn disable_swv(core: &mut Core) -> Result<(), Error> {
    let mut demcr = Demcr(core.read_word_32(Demcr::ADDRESS)?);
    demcr.set_dwtena(false);
    core.write_word_32(Demcr::ADDRESS, demcr.into())?;
    Ok(())
}
