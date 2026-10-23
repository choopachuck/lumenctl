//! MQTT bridge with Home Assistant discovery.
//!
//! Topics (`<base>` = `<base_topic>/<node_id>`):
//! ```text
//! <base>/status                    online / offline (LWT)
//! <base>/<monitor>/state           {"state":"ON","brightness":40,"contrast":50,"input":"HDMI1",...}
//! <base>/<monitor>/<field>/set     light (JSON) | contrast | volume | red | green | blue | input | color_preset | display_mode | power
//! <base>/command                   raw command line, e.g. "MateView b 30"
//! <base>/rescan                    any payload
//! ```

use std::collections::HashSet;
use std::time::Duration;

use lumen_core::config::Mqtt as MqttConfig;
use lumen_core::model::{MonitorSnapshot, PowerState};
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
    published: HashSet<String>,
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
    let mut bridge = Bridge { engine: engine.clone(), client, cfg, node, base, published: HashSet::new() };
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
        let _ = self
            .client
            .subscribe(format!("{}/status", self.cfg.discovery_prefix), QoS::AtLeastOnce)
            .await;
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

    async fn publish_all(&mut self, monitors: &[MonitorSnapshot]) {
        let p = &self.cfg.discovery_prefix;
        let n = &self.node;
        let b = &self.base;
        let avail = json!([{ "topic": format!("{b}/status") }]);

        let hub = [
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
        ];
        for (topic, payload) in hub {
            self.publish(topic, payload.to_string(), true).await;
        }

        let mut current = HashSet::new();
        for m in monitors {
            for (topic, payload) in self.discovery(m) {
                current.insert(topic.clone());
                self.publish(topic, payload.to_string(), true).await;
            }
            self.publish_state(m).await;
        }
        // Remove entities of monitors that disappeared.
        for stale in self.published.difference(&current) {
            self.publish(stale.clone(), Vec::new(), true).await;
        }
        self.published = current;
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
        self.publish(format!("{}/{}/state", self.base, m.info.key), state.to_string(), true).await;
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
    use super::*;
    use lumen_core::command::Value as V;

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
}
