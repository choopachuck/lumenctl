use crate::model::{MonitorInfo, VcpValue};

pub mod mock;
#[cfg(windows)]
pub mod windows;

/// A connection to one physical monitor. All calls are blocking and slow (DDC/CI needs
/// ~40-50 ms per transaction), so the engine drives each device from its own thread.
pub trait Device: Send {
    fn get_vcp(&mut self, code: u8) -> anyhow::Result<VcpValue>;
    fn set_vcp(&mut self, code: u8, value: u32) -> anyhow::Result<()>;
    fn capabilities(&mut self) -> anyhow::Result<String>;
    /// Asks the monitor to persist current settings to its NVRAM.
    fn save_settings(&mut self) -> anyhow::Result<()>;
}

pub struct Discovered {
    pub info: MonitorInfo,
    pub device: Box<dyn Device>,
}

pub trait Backend: Send + Sync {
    fn enumerate(&self) -> anyhow::Result<Vec<Discovered>>;
}

#[cfg(windows)]
pub fn default_backend() -> Box<dyn Backend> {
    Box::new(windows::WindowsBackend)
}

#[cfg(not(windows))]
pub fn default_backend() -> Box<dyn Backend> {
    Box::new(mock::MockBackend::with_monitors(2))
}
