//! MQTT bridge with Home Assistant discovery and the HACS integration contract.
//!
//! Topics (`<base>` = `<base_topic>/<node_id>`):
//! ```text
//! <base>/status                    online / offline (LWT)
//! <base>/info                      {"node_id","name","sw_version"}
//! <base>/<monitor>/info            monitor description for the HACS integration; empty payload removes it
//! <base>/<monitor>/state           {"state":"ON","brightness":40,"contrast":50,"input":"HDMI1",...}
//! <base>/<monitor>/<field>/set     light (JSON) | contrast | volume | red | green | blue | input | color_preset | display_mode | power
//! <base>/command                   raw command line, e.g. "MateView b 30"
//! <base>/rescan                    any payload
//! ```

use std::collections::HashSet;
use std::time::Duration;

use lumen_core::config::Mqtt as MqttConfig;
use lumen_core::model::{MonitorSnapshot, NamedValue, PowerState};
use lumen_core::patch::{MonitorPatch, NameOrValue, ValueSpec};
use lumen_core::{Engine, Event};
use rumqttc::{AsyncClient, Event as MqttEvent, LastWill, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

pub fn node_id(cfg: &MqttConfig) -> String {
    let raw = cfg
        .node_id
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "pc".into());
    lumen_core::model::key_from_id(&raw)
}

struct Bridge {
    engine: Engine,
    client: AsyncClient,
    cfg: MqttConfig,
    node: String,
    base: String,
    /// Discovery config topics published for the current monitor set.
    published: HashSet<String>,
    /// Monitor keys that currently have retained `state` and `info`.
    monitor_keys: HashSet<String>,
}

pub async fn run(engine: Engine, cfg: MqttConfig) -> anyhow::Result<()> {
    let node = node_id(&cfg);
    let base = format!("{}/{}", cfg.base_topic.trim_end_matches('/'), node);
    let mut opts = MqttOptions::new(format!("lumenctl-{node}"), cfg.host.clone(), cfg.port);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_last_will(LastWill::new(format!("{base}/status"), "offline", QoS::AtLeastOnce, true));
    if let Some(user) = cfg.username.clone().filter(|u| !u.is_empty()) {
        opts.set_credentials(user, cfg.password.clone().unwrap_or_default());
    }
    let (client, mut eventloop) = AsyncClient::new(opts, 128);
    let mut bridge = Bridge {
        engine: engine.clone(),
        client,
        cfg,
        node,
        base,
        published: HashSet::new(),
        monitor_keys: HashSet::new(),
    };
    let mut events = engine.subscribe();

    loop {
        tokio::select! {
            ev = eventloop.poll() => match ev {
                Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                    info!("mqtt connected to {}:{}", bridge.cfg.host, bridge.cfg.port);
                    bridge.on_connect().await;
                }
                Ok(MqttEvent::Incoming(Packet::Publish(p))) => {
                    let payload = String::from_utf8_lossy(&p.payload).to_string();
                    bridge.on_message(&p.topic, &payload).await;
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("mqtt: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            },
            ev = events.recv() => match ev {
                Ok(Event::MonitorUpdated { monitor }) => bridge.publish_state(&monitor).await,
                Ok(Event::MonitorsChanged { monitors }) => bridge.publish_all(&monitors).await,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => bridge.publish_all(&engine.monitors()).await,
                Err(RecvError::Closed) => return Ok(()),
            },
        }
    }
}

impl Bridge {
    async fn publish(&self, topic: String, payload: impl Into<Vec<u8>>, retain: bool) {
        if let Err(e) = self.client.publish(topic, QoS::AtLeastOnce, retain, payload).await {
            warn!("mqtt publish: {e}");
        }
    }

    async fn on_connect(&mut self) {
        let b = &self.base;
        for t in [format!("{b}/+/+/set"), format!("{b}/command"), format!("{b}/rescan")] {
            let _ = self.client.subscribe(t, QoS::AtLeastOnce).await;
        }
        if self.cfg.discovery {
            let _ = self
                .client
                .subscribe(format!("{}/status", self.cfg.discovery_prefix), QoS::AtLeastOnce)
                .await;
        }
        self.publish(format!("{b}/status"), "online", true).await;
        self.publish_all(&self.engine.monitors()).await;
    }

    fn hub_device(&self) -> Value {
        json!({
            "identifiers": [format!("lumenctl_{}", self.node)],
            "name": format!("lumenctl ({})", self.node),
            "manufacturer": "lumenctl",
            "sw_version": env!("CARGO_PKG_VERSION"),
        })
    }

    fn hub_discovery(&self) -> Vec<(String, Value)> {
        let p = &self.cfg.discovery_prefix;
        let n = &self.node;
        let b = &self.base;
        let avail = json!([{ "topic": format!("{b}/status") }]);
        vec![
            (
                format!("{p}/text/{n}/command/config"),
                json!({
                    "name": "Command", "unique_id": format!("lumenctl_{n}_command"),
                    "command_topic": format!("{b}/command"), "icon": "mdi:console-line",
                    "availability": avail, "device": self.hub_device(),
                }),
            ),
            (
                format!("{p}/button/{n}/rescan/config"),
                json!({
                    "name": "Rescan monitors", "unique_id": format!("lumenctl_{n}_rescan"),
                    "command_topic": format!("{b}/rescan"), "icon": "mdi:monitor-shimmer",
                    "availability": avail, "device": self.hub_device(),
                }),
            ),
        ]
    }

    async fn publish_all(&mut self, monitors: &[MonitorSnapshot]) {
        let info_topic = format!("{}/info", self.base);
        let info_body = hub_info_payload(&self.node, env!("CARGO_PKG_VERSION")).to_string();
        self.publish(info_topic, info_body, true).await;

        let mut current = HashSet::new();
        for (topic, payload) in self.hub_discovery() {
            current.insert(topic.clone());
            self.publish(topic, discovery_payload(self.cfg.discovery, &payload), true).await;
        }

        let mut current_keys = HashSet::new();
        for m in monitors {
            current_keys.insert(m.info.key.clone());
            for (topic, payload) in self.discovery(m) {
                current.insert(topic.clone());
                self.publish(topic, discovery_payload(self.cfg.discovery, &payload), true).await;
            }
            self.publish_state(m).await;
        }
        let stale_discovery: Vec<String> = self.published.difference(&current).cloned().collect();
        for stale in stale_discovery {
            self.publish(stale, Vec::new(), true).await;
        }
        for topic in disappeared_monitor_topics(&self.base, &self.monitor_keys, &current_keys) {
            self.publish(topic, Vec::new(), true).await;
        }
        self.published = current;
        self.monitor_keys = current_keys;
    }

    fn discovery(&self, m: &MonitorSnapshot) -> Vec<(String, Value)> {
        let p = &self.cfg.discovery_prefix;
        let n = &self.node;
        let k = &m.info.key;
        let t = format!("{}/{k}", self.base);
        let device = json!({
            "identifiers": [format!("lumenctl_{n}_{k}")],
            "name": m.display_name,
            "manufacturer": m.info.manufacturer_id,
            "model": m.info.name,
            "serial_number": m.info.serial,
            "via_device": format!("lumenctl_{n}"),
        });
        let common = |unique: &str| {
            json!({
                "unique_id": format!("lumenctl_{n}_{k}_{unique}"),
                "state_topic": format!("{t}/state"),
                "availability": [{ "topic": format!("{}/status", self.base) }],
                "device": device,
            })
        };
        let merge = |mut a: Value, b: Value| {
            a.as_object_mut().unwrap().extend(b.as_object().unwrap().clone());
            a
        };

        let mut out = vec![(
            format!("{p}/light/{n}/{k}/config"),
            merge(common("light"), json!({
                "name": null, "schema": "json", "brightness": true, "brightness_scale": 100,
                "command_topic": format!("{t}/light/set"), "icon": "mdi:monitor",
            })),
        )];

        let number = |field: &str, label: &str, icon: &str, max: u32, enabled: bool| {
            (
                format!("{p}/number/{n}/{k}_{field}/config"),
                merge(common(field), json!({
                    "name": label, "min": 0, "max": max, "step": 1, "mode": "slider",
                    "command_topic": format!("{t}/{field}/set"),
                    "value_template": format!("{{{{ value_json.{field} }}}}"),
                    "icon": icon, "enabled_by_default": enabled,
                })),
            )
        };
        let max_of = |v: Option<lumen_core::VcpValue>| v.map(|v| v.max).filter(|m| *m > 0).unwrap_or(100);
        if m.contrast.is_some() {
            out.push(number("contrast", "Contrast", "mdi:contrast-box", max_of(m.contrast), true));
        }
        if m.volume.is_some() {
            out.push(number("volume", "Speaker volume", "mdi:volume-high", max_of(m.volume), true));
        }
        for (field, label, v) in [("red", "Red", m.red), ("green", "Green", m.green), ("blue", "Blue", m.blue)] {
            if v.is_some() {
                out.push(number(field, label, "mdi:palette", max_of(v), false));
            }
        }

        let select = |field: &str, label: &str, icon: &str, options: Vec<String>| {
            (
                format!("{p}/select/{n}/{k}_{field}/config"),
                merge(common(field), json!({
                    "name": label, "options": options,
                    "command_topic": format!("{t}/{field}/set"),
                    "value_template": format!("{{{{ value_json.{field} }}}}"),
                    "icon": icon,
                })),
            )
        };
        let names = |v: &[lumen_core::model::NamedValue]| v.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        if m.input.is_some() {
            out.push(select("input", "Input", "mdi:video-input-hdmi", names(&m.supported_inputs)));
        }
        if m.color_preset.is_some() {
            out.push(select("color_preset", "Color preset", "mdi:palette-swatch", names(&m.supported_color_presets)));
        }
        if m.display_mode.is_some() {
            out.push(select("display_mode", "Display mode", "mdi:movie-open", names(&m.supported_display_modes)));
        }
        out
    }

    async fn publish_state(&self, m: &MonitorSnapshot) {
        let on = m.power.map_or(true, |p| p == PowerState::On);
        let state = json!({
            "state": if on { "ON" } else { "OFF" },
            "brightness": m.brightness.map(|v| v.current),
            "contrast": m.contrast.map(|v| v.current),
            "volume": m.volume.map(|v| v.current),
            "red": m.red.map(|v| v.current),
            "green": m.green.map(|v| v.current),
            "blue": m.blue.map(|v| v.current),
            "input": m.input.as_ref().map(|v| v.name.clone()),
            "color_preset": m.color_preset.as_ref().map(|v| v.name.clone()),
            "display_mode": m.display_mode.as_ref().map(|v| v.name.clone()),
            "ddc_ok": m.ddc_ok,
        });
        let key = &m.info.key;
        self.publish(format!("{}/{key}/state", self.base), state.to_string(), true).await;
        let limits = slider_limits(&self.engine, &m.info.id);
        self.publish(format!("{}/{key}/info", self.base), monitor_info_payload(m, &limits).to_string(), true).await;
    }

    async fn on_message(&mut self, topic: &str, payload: &str) {
        if topic == format!("{}/status", self.cfg.discovery_prefix) {
            if payload == "online" {
                self.publish_all(&self.engine.monitors()).await;
            }
            return;
        }
        let engine = self.engine.clone();
        let Some(rest) = topic.strip_prefix(&format!("{}/", self.base)) else { return };
        let payload = payload.trim().to_string();

        if rest == "command" {
            spawn(move || engine.execute(&payload).map(drop));
            return;
        }
        if rest == "rescan" {
            spawn(move || engine.rescan());
            return;
        }
        let parts: Vec<&str> = rest.split('/').collect();
        let [key, field, "set"] = parts.as_slice() else { return };
        let Some(patch) = patch_for(field, &payload) else {
            warn!("mqtt: cannot handle {topic} = {payload}");
            return;
        };
        let key = key.to_string();
        spawn(move || {
            let m = engine.resolve(&key).ok_or_else(|| anyhow::anyhow!("monitor {key} not found"))?;
            engine.apply_patch(&m, &patch)
        });
    }
}

struct SliderLimits {
    min_brightness: Option<u32>,
    max_brightness: Option<u32>,
    min_contrast: Option<u32>,
    max_contrast: Option<u32>,
}

fn slider_limits(engine: &lumen_core::Engine, monitor_id: &str) -> SliderLimits {
    let cfg = engine.config();
    let mc = cfg.monitor(monitor_id);
    SliderLimits {
        min_brightness: mc.and_then(|c| c.min_brightness),
        max_brightness: mc.and_then(|c| c.max_brightness),
        min_contrast: mc.and_then(|c| c.min_contrast),
        max_contrast: mc.and_then(|c| c.max_contrast),
    }
}

/// Retained discovery body. When discovery is off the payload is empty so Home
/// Assistant drops a config it already stored.
fn discovery_payload(enabled: bool, body: &Value) -> Vec<u8> {
    if enabled { body.to_string().into_bytes() } else { Vec::new() }
}

fn hub_info_payload(node: &str, version: &str) -> Value {
    json!({
        "node_id": node,
        "name": format!("lumenctl ({node})"),
        "sw_version": version,
    })
}

fn power_name(power: Option<PowerState>) -> &'static str {
    match power {
        Some(PowerState::Off) => "off",
        Some(PowerState::Standby) => "standby",
        _ => "on",
    }
}

/// `min`/`max` follow the monitor config and the VCP maximum. The window is
/// widened to include `current` so a Home Assistant slider can display it.
fn range_json(value: Option<lumen_core::VcpValue>, min_cfg: Option<u32>, max_cfg: Option<u32>) -> Value {
    let Some(v) = value else { return Value::Null };
    let reported = if v.max == 0 { 100 } else { v.max };
    let mut min = min_cfg.unwrap_or(0).min(reported);
    let mut max = max_cfg.unwrap_or(reported).min(reported);
    if max < min {
        max = min;
    }
    if v.current < min {
        min = v.current;
    }
    if v.current > max {
        max = v.current;
    }
    json!({"current": v.current, "min": min, "max": max})
}

fn name_list(values: &[NamedValue]) -> Vec<String> {
    values.iter().map(|v| v.name.clone()).collect()
}

fn monitor_info_payload(m: &MonitorSnapshot, limits: &SliderLimits) -> Value {
    json!({
        "key": m.info.key,
        "id": m.info.id,
        "name": m.display_name,
        "manufacturer": m.info.manufacturer_id,
        "model": m.info.name,
        "serial_number": m.info.serial,
        "ddc_ok": m.ddc_ok,
        "power": power_name(m.power),
        "brightness": range_json(m.brightness, limits.min_brightness, limits.max_brightness),
        "contrast": range_json(m.contrast, limits.min_contrast, limits.max_contrast),
        "volume": range_json(m.volume, None, None),
        "red": range_json(m.red, None, None),
        "green": range_json(m.green, None, None),
        "blue": range_json(m.blue, None, None),
        "input": m.input.as_ref().map(|v| v.name.clone()),
        "inputs": name_list(&m.supported_inputs),
        "color_preset": m.color_preset.as_ref().map(|v| v.name.clone()),
        "color_presets": name_list(&m.supported_color_presets),
        "display_mode": m.display_mode.as_ref().map(|v| v.name.clone()),
        "display_modes": name_list(&m.supported_display_modes),
    })
}

fn disappeared_monitor_topics(base: &str, previous: &HashSet<String>, current: &HashSet<String>) -> Vec<String> {
    let mut topics: Vec<String> = previous
        .difference(current)
        .flat_map(|key| [format!("{base}/{key}/state"), format!("{base}/{key}/info")])
        .collect();
    topics.sort();
    topics
}

fn spawn(f: impl FnOnce() -> anyhow::Result<()> + Send + 'static) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = f() {
            warn!("mqtt command failed: {e:#}");
        }
    });
}

fn patch_for(field: &str, payload: &str) -> Option<MonitorPatch> {
    let num = || payload.parse::<f64>().ok().map(|v| ValueSpec(lumen_core::command::Value::Abs(v.round().max(0.0) as u32)));
    let name = || Some(NameOrValue::Name(payload.to_string()));
    let mut p = MonitorPatch::default();
    match field {
        "light" => {
            let v: Value = serde_json::from_str(payload).ok()?;
            match v.get("state").and_then(Value::as_str) {
                Some("OFF") => p.power = Some(false),
                Some("ON") if v.get("brightness").is_none() => p.power = Some(true),
                _ => {}
            }
            if let Some(b) = v.get("brightness").and_then(Value::as_f64) {
                p.brightness = Some(ValueSpec(lumen_core::command::Value::Abs(b.round().max(0.0) as u32)));
            }
        }
        "brightness" => p.brightness = Some(num()?),
        "contrast" => p.contrast = Some(num()?),
        "volume" => p.volume = Some(num()?),
        "red" => p.red = Some(num()?),
        "green" => p.green = Some(num()?),
        "blue" => p.blue = Some(num()?),
        "input" => p.input = name(),
        "color_preset" => p.color_preset = name(),
        "display_mode" => p.display_mode = name(),
        "power" => p.power = Some(matches!(payload.to_ascii_uppercase().as_str(), "ON" | "1" | "TRUE")),
        _ => return None,
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use lumen_core::command::Value as V;
    use serde_json::json;

    #[test]
    fn light_payloads() {
        let p = patch_for("light", r#"{"state":"ON","brightness":42}"#).unwrap();
        assert_eq!(p.brightness, Some(ValueSpec(V::Abs(42))));
        assert_eq!(p.power, None);
        let p = patch_for("light", r#"{"state":"OFF"}"#).unwrap();
        assert_eq!(p.power, Some(false));
        let p = patch_for("light", r#"{"state":"ON"}"#).unwrap();
        assert_eq!(p.power, Some(true));
    }

    #[test]
    fn scalar_payloads() {
        assert_eq!(patch_for("contrast", "55.0").unwrap().contrast, Some(ValueSpec(V::Abs(55))));
        assert_eq!(patch_for("input", "HDMI1").unwrap().input, Some(NameOrValue::Name("HDMI1".into())));
        assert!(patch_for("bogus", "1").is_none());
    }

    fn sample_monitor() -> MonitorSnapshot {
        use lumen_core::model::MonitorInfo;
        MonitorSnapshot {
            info: MonitorInfo {
                id: "MATE#1".into(),
                key: "mate".into(),
                name: "MateView".into(),
                manufacturer_id: Some("HWV".into()),
                serial: Some("ABC".into()),
                ..MonitorInfo::default()
            },
            display_name: "MateView".into(),
            ddc_ok: true,
            brightness: Some(lumen_core::VcpValue { current: 40, max: 100 }),
            contrast: Some(lumen_core::VcpValue { current: 50, max: 100 }),
            red: None,
            green: None,
            blue: None,
            volume: None,
            input: Some(NamedValue { value: 0x11, name: "HDMI1".into() }),
            color_preset: Some(NamedValue { value: 5, name: "6500K".into() }),
            display_mode: None,
            power: Some(PowerState::On),
            supported_inputs: vec![
                NamedValue { value: 0x11, name: "HDMI1".into() },
                NamedValue { value: 0x0F, name: "DisplayPort1".into() },
            ],
            supported_color_presets: vec![
                NamedValue { value: 5, name: "6500K".into() },
                NamedValue { value: 11, name: "User1".into() },
            ],
            supported_display_modes: vec![],
            last_error: None,
        }
    }

    #[test]
    fn monitor_info_shape() {
        let payload = monitor_info_payload(
            &sample_monitor(),
            &SliderLimits {
                min_brightness: Some(10),
                max_brightness: Some(80),
                min_contrast: None,
                max_contrast: None,
            },
        );
        assert_eq!(payload["key"], json!("mate"));
        assert_eq!(payload["name"], json!("MateView"));
        assert_eq!(payload["manufacturer"], json!("HWV"));
        assert_eq!(payload["model"], json!("MateView"));
        assert_eq!(payload["serial_number"], json!("ABC"));
        assert_eq!(payload["ddc_ok"], json!(true));
        assert_eq!(payload["power"], json!("on"));
        assert_eq!(payload["brightness"], json!({"current": 40, "min": 10, "max": 80}));
        assert_eq!(payload["contrast"], json!({"current": 50, "min": 0, "max": 100}));
        assert!(payload["volume"].is_null());
        assert!(payload["red"].is_null());
        assert!(payload["green"].is_null());
        assert!(payload["blue"].is_null());
        assert_eq!(payload["input"], json!("HDMI1"));
        assert_eq!(payload["inputs"], json!(["HDMI1", "DisplayPort1"]));
        assert_eq!(payload["color_presets"], json!(["6500K", "User1"]));
        assert!(payload["display_mode"].is_null());
        assert_eq!(payload["display_modes"], json!([]));
    }

    #[test]
    fn monitor_info_widens_slider_and_names_power() {
        let mut monitor = sample_monitor();
        monitor.brightness = Some(lumen_core::VcpValue { current: 5, max: 0 });
        monitor.power = Some(PowerState::Off);
        let low = monitor_info_payload(
            &monitor,
            &SliderLimits {
                min_brightness: Some(10),
                max_brightness: Some(80),
                min_contrast: None,
                max_contrast: None,
            },
        );
        assert_eq!(low["brightness"]["current"], 5);
        assert_eq!(low["brightness"]["min"], 5);
        assert_eq!(low["brightness"]["max"], 80);
        assert_eq!(low["power"], json!("off"));

        monitor.power = None;
        let unknown = monitor_info_payload(
            &monitor,
            &SliderLimits { min_brightness: None, max_brightness: None, min_contrast: None, max_contrast: None },
        );
        assert_eq!(unknown["power"], json!("on"));
        assert_eq!(unknown["brightness"]["max"], 100);
    }

    #[test]
    fn discovery_disabled_publishes_an_empty_config() {
        let body = json!({"name": "Contrast"});
        assert!(discovery_payload(false, &body).is_empty());
        let enabled = discovery_payload(true, &body);
        assert_eq!(enabled, body.to_string().into_bytes());
    }

    #[test]
    fn disappeared_monitors_clear_state_and_info() {
        let previous: HashSet<String> = ["mate".into(), "gone".into()].into();
        let current: HashSet<String> = ["mate".into()].into();
        assert_eq!(
            disappeared_monitor_topics("lumenctl/desktop", &previous, &current),
            vec![
                "lumenctl/desktop/gone/info".to_string(),
                "lumenctl/desktop/gone/state".to_string(),
            ]
        );
    }

    #[test]
    fn hub_info_names_the_pc() {
        let v = hub_info_payload("desktop", "1.0.0");
        assert_eq!(v["node_id"], json!("desktop"));
        assert_eq!(v["name"], json!("lumenctl (desktop)"));
        assert_eq!(v["sw_version"], json!("1.0.0"));
    }
}
