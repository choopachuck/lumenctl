use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::caps::Capabilities;
use crate::vcp::{self, Feature};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VcpValue {
    pub current: u32,
    pub max: u32,
}

impl VcpValue {
    /// Value scaled to 0..=100 (most monitors already report max = 100).
    pub fn percent(&self) -> u32 {
        if self.max == 0 || self.max == 100 {
            self.current.min(100)
        } else {
            ((self.current as f64 / self.max as f64) * 100.0).round() as u32
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    External,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Static description of a physical monitor, gathered on discovery.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonitorInfo {
    /// Stable identifier: `<PnP id>#<instance>` (same format as ClickMonitorDDC's `mUID`).
    pub id: String,
    /// URL/MQTT-safe form of `id` (`hwv6e22_5_26d47761_0_uid4355`).
    pub key: String,
    /// 1-based position in the monitor list.
    pub index: usize,
    /// Friendly name from the OS / EDID (`PL2779Q`, `MateView`).
    pub name: String,
    /// User-defined name, if any.
    pub alias: Option<String>,
    pub manufacturer_id: Option<String>,
    pub product_code: Option<String>,
    pub serial: Option<String>,
    pub year: Option<u16>,
    pub week: Option<u8>,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub gdi_device: String,
    pub description: String,
    pub rect: Rect,
    pub primary: bool,
    pub connection: Connection,
    pub output: Option<String>,
}

pub fn key_from_id(id: &str) -> String {
    let mut key = String::with_capacity(id.len());
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            key.push(c.to_ascii_lowercase());
        } else if !key.ends_with('_') {
            key.push('_');
        }
    }
    key.trim_matches('_').to_string()
}

impl MonitorInfo {
    pub fn display_name(&self) -> &str {
        self.alias.as_deref().filter(|a| !a.is_empty()).unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorState {
    /// Last known value of every VCP code read or written.
    pub vcp: BTreeMap<u8, VcpValue>,
    pub capabilities: Option<Capabilities>,
    /// True if the monitor answered DDC/CI on the last poll.
    pub ddc_ok: bool,
    pub last_error: Option<String>,
}

impl MonitorState {
    pub fn get(&self, code: u8) -> Option<VcpValue> {
        self.vcp.get(&code).copied()
    }
}

/// API-friendly view of a monitor.
#[derive(Debug, Clone, Serialize)]
pub struct MonitorSnapshot {
    #[serde(flatten)]
    pub info: MonitorInfo,
    pub display_name: String,
    pub ddc_ok: bool,
    pub brightness: Option<VcpValue>,
    pub contrast: Option<VcpValue>,
    pub red: Option<VcpValue>,
    pub green: Option<VcpValue>,
    pub blue: Option<VcpValue>,
    pub volume: Option<VcpValue>,
    pub input: Option<NamedValue>,
    pub color_preset: Option<NamedValue>,
    pub display_mode: Option<NamedValue>,
    pub power: Option<PowerState>,
    pub supported_inputs: Vec<NamedValue>,
    pub supported_color_presets: Vec<NamedValue>,
    pub supported_display_modes: Vec<NamedValue>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedValue {
    pub value: u32,
    pub name: String,
}

impl NamedValue {
    pub fn from_table(table: &[(u32, &'static str)], value: u32) -> Self {
        Self {
            value,
            name: vcp::name_of(table, value)
                .map(str::to_string)
                .unwrap_or_else(|| format!("0x{value:02X}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerState {
    On,
    Standby,
    Off,
}

impl PowerState {
    pub fn from_vcp(v: u32) -> Self {
        match v {
            0x01 => PowerState::On,
            0x05 => PowerState::Off,
            _ => PowerState::Standby,
        }
    }
}

impl MonitorSnapshot {
    pub fn build(info: &MonitorInfo, state: &MonitorState, use_6b: bool) -> Self {
        let g = |f: Feature| state.get(f.code(use_6b));
        let named = |code: u8, table: &[(u32, &'static str)]| {
            state.get(code).map(|v| NamedValue::from_table(table, v.current & 0xFF))
        };
        let supported = |code: u8, table: &[(u32, &'static str)]| -> Vec<NamedValue> {
            let from_caps: Vec<u32> = state
                .capabilities
                .as_ref()
                .map(|c| c.values(code).to_vec())
                .unwrap_or_default();
            if from_caps.is_empty() {
                table.iter().map(|(v, _)| NamedValue::from_table(table, *v)).collect()
            } else {
                from_caps.into_iter().map(|v| NamedValue::from_table(table, v)).collect()
            }
        };
        Self {
            info: info.clone(),
            display_name: info.display_name().to_string(),
            ddc_ok: state.ddc_ok,
            brightness: g(Feature::Brightness),
            contrast: g(Feature::Contrast),
            red: g(Feature::Red),
            green: g(Feature::Green),
            blue: g(Feature::Blue),
            volume: g(Feature::Volume),
            input: named(vcp::INPUT_SOURCE, vcp::INPUTS),
            color_preset: named(vcp::COLOR_PRESET, vcp::COLOR_PRESETS),
            display_mode: named(vcp::DISPLAY_MODE, vcp::DISPLAY_MODES),
            power: state.get(vcp::POWER_MODE).map(|v| PowerState::from_vcp(v.current)),
            supported_inputs: supported(vcp::INPUT_SOURCE, vcp::INPUTS),
            supported_color_presets: supported(vcp::COLOR_PRESET, vcp::COLOR_PRESETS),
            supported_display_modes: supported(vcp::DISPLAY_MODE, vcp::DISPLAY_MODES),
            last_error: state.last_error.clone(),
        }
    }
}
