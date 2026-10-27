use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub api: Api,
    pub mqtt: Mqtt,
    pub tray: Tray,
    #[serde(rename = "monitor")]
    pub monitors: Vec<MonitorConfig>,
    /// Custom entries in the tray context menu.
    #[serde(rename = "menu")]
    pub menu: Vec<MenuEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Commands without a monitor selector target the monitor under the mouse cursor.
    pub target_monitor_under_cursor: bool,
    /// Monitor (id, name or alias) used by commands without a selector.
    pub default_monitor: Option<String>,
    /// Re-apply last brightness/contrast on start and after monitors wake up.
    pub restore_values: bool,
    /// Read the value back after each write.
    pub verify_writes: bool,
    /// Ask the monitor to persist writes to NVRAM (slower, for monitors that forget values).
    pub save_to_nvram: bool,
    /// Apply brightness/contrast changes to all monitors at once.
    pub same_values_for_all: bool,
    pub start_with_windows: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            target_monitor_under_cursor: false,
            default_monitor: None,
            restore_values: false,
            verify_writes: false,
            save_to_nvram: false,
            same_values_for_all: false,
            start_with_windows: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Api {
    pub enabled: bool,
    /// Use `0.0.0.0:7685` to allow Home Assistant on another host; set a token in that case.
    pub bind: String,
    /// Bearer token required for every request when set.
    pub token: Option<String>,
}

impl Default for Api {
    fn default() -> Self {
        Self { enabled: true, bind: "127.0.0.1:7685".into(), token: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mqtt {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub discovery_prefix: String,
    pub base_topic: String,
    /// Device identifier in Home Assistant; defaults to the computer name.
    pub node_id: Option<String>,
    /// Publish Home Assistant MQTT discovery. Turn this off when the lumenctl
    /// HACS integration is installed, otherwise every entity appears twice.
    #[serde(default = "default_true")]
    pub discovery: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Mqtt {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "homeassistant.local".into(),
            port: 1883,
            username: None,
            password: None,
            discovery_prefix: "homeassistant".into(),
            base_topic: "lumenctl".into(),
            node_id: None,
            discovery: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Tray {
    pub enabled: bool,
    pub wheel_step_brightness: u32,
    pub wheel_step_contrast: u32,
    pub wheel_step_volume: u32,
    /// Middle click on a brightness icon toggles between these two values.
    pub middle_click_brightness: [u32; 2],
    pub middle_click_contrast: [u32; 2],
    pub font: String,
    /// `#RRGGBB`
    pub brightness_color: String,
    pub contrast_color: String,
    pub volume_color: String,
    pub show_system_volume_icon: bool,
    /// Hide the control window when it loses focus (flyout behaviour).
    pub hide_window_on_blur: bool,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            enabled: true,
            wheel_step_brightness: 5,
            wheel_step_contrast: 10,
            wheel_step_volume: 5,
            middle_click_brightness: [0, 50],
            middle_click_contrast: [0, 50],
            font: "Segoe UI".into(),
            brightness_color: "#FFD60A".into(),
            contrast_color: "#30D158".into(),
            volume_color: "#64D2FF".into(),
            show_system_volume_icon: false,
            hide_window_on_blur: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MonitorConfig {
    /// Stable id as reported by `/api/monitors`.
    pub id: String,
    pub alias: Option<String>,
    /// Skip DDC/CI for this device entirely (e.g. AV receivers).
    pub ignore: bool,
    /// Use VCP 0x6B instead of 0x10 for brightness.
    pub use_6b: bool,
    pub show_brightness_icon: Option<bool>,
    pub show_contrast_icon: Option<bool>,
    pub min_brightness: Option<u32>,
    pub max_brightness: Option<u32>,
    pub min_contrast: Option<u32>,
    pub max_contrast: Option<u32>,
    /// Tray icon typeface. Empty means the global tray font.
    pub font: Option<String>,
    /// `#RRGGBB` for this monitor's brightness icon. Empty means the global brightness color.
    pub brightness_color: Option<String>,
    /// `#RRGGBB` for this monitor's contrast icon. Empty means the global contrast color.
    pub contrast_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MenuEntry {
    pub label: String,
    pub command: String,
}

impl Config {
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lumenctl")
            .join("config.toml")
    }

    pub fn load(path: &Path) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn monitor(&self, id: &str) -> Option<&MonitorConfig> {
        self.monitors.iter().find(|m| m.id == id)
    }

    pub fn monitor_mut(&mut self, id: &str) -> &mut MonitorConfig {
        if let Some(i) = self.monitors.iter().position(|m| m.id == id) {
            &mut self.monitors[i]
        } else {
            self.monitors.push(MonitorConfig { id: id.to_string(), ..Default::default() });
            self.monitors.last_mut().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.monitor_mut("HWV6E22#5&26d47761&0&UID4355").alias = Some("MateView".into());
        c.menu.push(MenuEntry { label: "Night".into(), command: "b 10 c 30".into() });
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.monitors[0].alias.as_deref(), Some("MateView"));
        assert_eq!(back.menu[0].command, "b 10 c 30");
        assert_eq!(back.api.bind, "127.0.0.1:7685");
    }

    #[test]
    fn empty_file_gives_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert!(c.api.enabled);
        assert!(c.mqtt.discovery);
        assert_eq!(c.tray.wheel_step_brightness, 5);
    }

    #[test]
    fn mqtt_without_discovery_field_keeps_discovery_on() {
        let c: Config = toml::from_str("[mqtt]\nenabled = true\n").unwrap();
        assert!(c.mqtt.enabled);
        assert!(c.mqtt.discovery);
        assert_eq!(c.mqtt.base_topic, "lumenctl");
    }
}
