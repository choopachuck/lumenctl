//! Declarative monitor changes shared by the HTTP/WS API, MQTT and the GUI.

use serde::{Deserialize, Deserializer, Serialize};

use crate::command::Value;
use crate::engine::{Engine, Monitor};
use crate::vcp::Feature;

/// Absolute number (`50`) or relative string (`"+10"`, `"-5"`, `"50"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ValueSpec(pub Value);

impl<'de> Deserialize<'de> for ValueSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(i64),
            Str(String),
        }
        let parse = |s: &str| -> Option<Value> {
            let s = s.trim();
            if let Some(r) = s.strip_prefix('+') {
                r.trim().parse().ok().map(Value::Rel)
            } else if let Some(r) = s.strip_prefix('-') {
                r.trim().parse::<i32>().ok().map(|v| Value::Rel(-v))
            } else {
                s.parse().ok().map(Value::Abs)
            }
        };
        match Raw::deserialize(d)? {
            Raw::Num(n) if n >= 0 => Ok(ValueSpec(Value::Abs(n as u32))),
            Raw::Num(n) => Err(serde::de::Error::custom(format!("negative value {n}"))),
            Raw::Str(s) => parse(&s)
                .map(ValueSpec)
                .ok_or_else(|| serde::de::Error::custom(format!("invalid value '{s}'"))),
        }
    }
}

/// Input / preset given by name (`"HDMI1"`) or raw VCP value (`17`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NameOrValue {
    Value(u32),
    Name(String),
}

impl NameOrValue {
    fn as_text(&self) -> String {
        match self {
            NameOrValue::Value(v) => v.to_string(),
            NameOrValue::Name(n) => n.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorPatch {
    pub brightness: Option<ValueSpec>,
    pub contrast: Option<ValueSpec>,
    pub red: Option<ValueSpec>,
    pub green: Option<ValueSpec>,
    pub blue: Option<ValueSpec>,
    pub volume: Option<ValueSpec>,
    pub input: Option<NameOrValue>,
    pub color_preset: Option<NameOrValue>,
    pub display_mode: Option<NameOrValue>,
    pub power: Option<bool>,
}

impl Engine {
    pub fn apply_patch(&self, m: &Monitor, p: &MonitorPatch) -> anyhow::Result<()> {
        if let Some(on) = p.power.filter(|on| *on) {
            self.set_power(m, on);
        }
        for (feature, spec) in [
            (Feature::Brightness, p.brightness),
            (Feature::Contrast, p.contrast),
            (Feature::Red, p.red),
            (Feature::Green, p.green),
            (Feature::Blue, p.blue),
            (Feature::Volume, p.volume),
        ] {
            if let Some(ValueSpec(v)) = spec {
                self.adjust(m, feature, v)?;
            }
        }
        if let Some(i) = &p.input {
            self.set_input(m, &i.as_text())?;
        }
        if let Some(c) = &p.color_preset {
            self.set_color_preset(m, &c.as_text())?;
        }
        if let Some(d) = &p.display_mode {
            self.set_display_mode(m, &d.as_text())?;
        }
        if p.power == Some(false) {
            self.set_power(m, false);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_values() {
        let p: MonitorPatch =
            serde_json::from_str(r#"{"brightness": 40, "contrast": "+5", "volume": "-10", "input": "HDMI1", "color_preset": 5}"#)
                .unwrap();
        assert_eq!(p.brightness, Some(ValueSpec(Value::Abs(40))));
        assert_eq!(p.contrast, Some(ValueSpec(Value::Rel(5))));
        assert_eq!(p.volume, Some(ValueSpec(Value::Rel(-10))));
        assert_eq!(p.input, Some(NameOrValue::Name("HDMI1".into())));
        assert_eq!(p.color_preset, Some(NameOrValue::Value(5)));
        assert!(serde_json::from_str::<MonitorPatch>(r#"{"bogus": 1}"#).is_err());
    }
}
