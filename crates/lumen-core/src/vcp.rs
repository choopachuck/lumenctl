//! MCCS VCP codes and the symbolic names used by the command language and the API.

pub const RESTORE_FACTORY_DEFAULTS: u8 = 0x04;
pub const RESTORE_FACTORY_COLOR: u8 = 0x08;
pub const BRIGHTNESS: u8 = 0x10;
pub const CONTRAST: u8 = 0x12;
pub const COLOR_PRESET: u8 = 0x14;
pub const RED_GAIN: u8 = 0x16;
pub const GREEN_GAIN: u8 = 0x18;
pub const BLUE_GAIN: u8 = 0x1A;
pub const INPUT_SOURCE: u8 = 0x60;
pub const SPEAKER_VOLUME: u8 = 0x62;
pub const AUDIO_MUTE: u8 = 0x8D;
pub const BACKLIGHT_WHITE: u8 = 0x6B;
pub const USAGE_TIME: u8 = 0xC0;
pub const POWER_MODE: u8 = 0xD6;
pub const DISPLAY_MODE: u8 = 0xDC;
pub const VERSION: u8 = 0xDF;

pub const POWER_ON: u32 = 0x01;
pub const POWER_OFF_DPM: u32 = 0x04;
pub const POWER_OFF_HARD: u32 = 0x05;

/// Codes polled on startup and on rescan for every monitor.
pub const POLLED: &[u8] = &[
    BRIGHTNESS,
    CONTRAST,
    RED_GAIN,
    GREEN_GAIN,
    BLUE_GAIN,
    SPEAKER_VOLUME,
    INPUT_SOURCE,
    COLOR_PRESET,
    DISPLAY_MODE,
    POWER_MODE,
];

/// Continuous features addressable by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Brightness,
    Contrast,
    Red,
    Green,
    Blue,
    Volume,
}

impl Feature {
    pub const ALL: [Feature; 6] = [
        Feature::Brightness,
        Feature::Contrast,
        Feature::Red,
        Feature::Green,
        Feature::Blue,
        Feature::Volume,
    ];

    pub fn code(self, use_6b: bool) -> u8 {
        match self {
            Feature::Brightness if use_6b => BACKLIGHT_WHITE,
            Feature::Brightness => BRIGHTNESS,
            Feature::Contrast => CONTRAST,
            Feature::Red => RED_GAIN,
            Feature::Green => GREEN_GAIN,
            Feature::Blue => BLUE_GAIN,
            Feature::Volume => SPEAKER_VOLUME,
        }
    }

    pub fn from_code(code: u8) -> Option<Feature> {
        Some(match code {
            BRIGHTNESS | BACKLIGHT_WHITE => Feature::Brightness,
            CONTRAST => Feature::Contrast,
            RED_GAIN => Feature::Red,
            GREEN_GAIN => Feature::Green,
            BLUE_GAIN => Feature::Blue,
            SPEAKER_VOLUME => Feature::Volume,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Brightness => "brightness",
            Feature::Contrast => "contrast",
            Feature::Red => "red",
            Feature::Green => "green",
            Feature::Blue => "blue",
            Feature::Volume => "volume",
        }
    }
}

pub const INPUTS: &[(u32, &str)] = &[
    (0x01, "VGA1"),
    (0x02, "VGA2"),
    (0x03, "DVI1"),
    (0x04, "DVI2"),
    (0x05, "Composite1"),
    (0x06, "Composite2"),
    (0x07, "SVideo1"),
    (0x08, "SVideo2"),
    (0x09, "Tuner1"),
    (0x0A, "Tuner2"),
    (0x0B, "Tuner3"),
    (0x0C, "Component1"),
    (0x0D, "Component2"),
    (0x0E, "Component3"),
    (0x0F, "DisplayPort1"),
    (0x10, "DisplayPort2"),
    (0x11, "HDMI1"),
    (0x12, "HDMI2"),
    (0x1B, "USB-C"),
];

pub const COLOR_PRESETS: &[(u32, &str)] = &[
    (0x01, "sRGB"),
    (0x02, "Native"),
    (0x03, "4000K"),
    (0x04, "5000K"),
    (0x05, "6500K"),
    (0x06, "7500K"),
    (0x07, "8200K"),
    (0x08, "9300K"),
    (0x09, "10000K"),
    (0x0A, "11500K"),
    (0x0B, "User1"),
    (0x0C, "User2"),
    (0x0D, "User3"),
];

pub const DISPLAY_MODES: &[(u32, &str)] = &[
    (0x00, "Standard"),
    (0x01, "Productivity"),
    (0x02, "Mixed"),
    (0x03, "Movie"),
    (0x04, "User"),
    (0x05, "Games"),
    (0x06, "Sports"),
    (0x07, "Professional"),
    (0x08, "StandardIntermediatePower"),
    (0x09, "StandardLowPower"),
    (0x0A, "Demonstration"),
    (0xF0, "DynamicContrast"),
];

pub fn name_of(table: &[(u32, &'static str)], value: u32) -> Option<&'static str> {
    table.iter().find(|(v, _)| *v == value).map(|(_, n)| *n)
}

/// Resolves a user supplied name (`hdmi`, `HDMI2`, `0x11`, `17`, `user`) against a table.
/// Exact match wins, otherwise the first entry starting with the given text.
pub fn lookup(table: &[(u32, &'static str)], text: &str) -> Option<u32> {
    if let Some(n) = parse_number(text) {
        return Some(n);
    }
    let norm = normalize(text);
    table
        .iter()
        .find(|(_, n)| normalize(n) == norm)
        .or_else(|| table.iter().find(|(_, n)| normalize(n).starts_with(&norm)))
        .map(|(v, _)| *v)
}

pub fn parse_number(text: &str) -> Option<u32> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        t.parse().ok()
    } else {
        None
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn vcp_name(code: u8) -> &'static str {
    match code {
        0x02 => "New Control Value",
        0x04 => "Restore Factory Defaults",
        0x05 => "Restore Factory Luminance/Contrast",
        0x06 => "Restore Factory Geometry",
        0x08 => "Restore Factory Color",
        0x0B => "Color Temperature Increment",
        0x0C => "Color Temperature Request",
        0x10 => "Brightness",
        0x12 => "Contrast",
        0x14 => "Color Preset",
        0x16 => "Red Gain",
        0x18 => "Green Gain",
        0x1A => "Blue Gain",
        0x52 => "Active Control",
        0x59..=0x5E => "6 Axis Saturation Control",
        0x60 => "Input Source",
        0x62 => "Audio: Speaker Volume",
        0x6B => "Backlight Level: White",
        0x6C => "Red Black Level",
        0x6E => "Green Black Level",
        0x70 => "Blue Black Level",
        0x72 => "Gamma",
        0x86 => "Display Scaling",
        0x87 => "Sharpness",
        0x8A => "Color Saturation",
        0x8D => "Audio Mute / Screen Blank",
        0x8F => "Audio Treble",
        0x91 => "Audio Bass",
        0x93 => "Audio Balance L/R",
        0xAA => "Screen Orientation",
        0xAC => "Horizontal Frequency",
        0xAE => "Vertical Frequency",
        0xB0 => "Settings",
        0xB2 => "Flat Panel Sub-Pixel Layout",
        0xB6 => "Display Technology Type",
        0xC0 => "Display Usage Time",
        0xC6 => "Application Enable Key",
        0xC8 => "Display Controller Type",
        0xC9 => "Display Firmware Level",
        0xCA => "OSD / Button Control",
        0xCC => "OSD Language",
        0xD6 => "Power Mode",
        0xDC => "Display Mode",
        0xDF => "VCP Version",
        0xE0..=0xFF => "Manufacturer Specific",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_names() {
        assert_eq!(lookup(INPUTS, "hdmi"), Some(0x11));
        assert_eq!(lookup(INPUTS, "HDMI2"), Some(0x12));
        assert_eq!(lookup(INPUTS, "DisplayPort1"), Some(0x0F));
        assert_eq!(lookup(INPUTS, "0x0f"), Some(0x0F));
        assert_eq!(lookup(COLOR_PRESETS, "user"), Some(0x0B));
        assert_eq!(lookup(COLOR_PRESETS, "6500K"), Some(0x05));
        assert_eq!(lookup(DISPLAY_MODES, "movie"), Some(0x03));
        assert_eq!(lookup(DISPLAY_MODES, "nope"), None);
    }
}
