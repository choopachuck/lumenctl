//! Core of lumenctl: monitor discovery, DDC/CI control, the command language and the event bus.

pub mod backend;
pub mod caps;
pub mod command;
pub mod config;
pub mod edid;
pub mod engine;
pub mod model;
pub mod patch;
pub mod platform;
pub mod vcp;

pub use config::Config;
pub use engine::{Engine, Event, ExecOutcome, Monitor};
pub use model::{MonitorInfo, MonitorSnapshot, VcpValue};
