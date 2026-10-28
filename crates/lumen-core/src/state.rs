//! Last brightness and contrast lumenctl successfully applied, per monitor.
//!
//! Kept out of `config.toml` so moving a slider does not rewrite settings.
//! Polled values are not stored: after sleep a monitor often answers with a factory
//! default, and that read must not erase the number we are about to put back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::vcp;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Saved {
    pub brightness: Option<u32>,
    pub contrast: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FileModel {
    #[serde(default)]
    monitor: Vec<Row>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Row {
    id: String,
    #[serde(default)]
    brightness: Option<u32>,
    #[serde(default)]
    contrast: Option<u32>,
}

pub struct Store {
    path: Option<PathBuf>,
    inner: Mutex<BTreeMap<String, Saved>>,
}

impl Store {
    pub fn open(path: Option<PathBuf>) -> Self {
        let inner = path.as_deref().map(load).unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    pub fn get(&self, id: &str) -> Saved {
        self.inner.lock().get(id).copied().unwrap_or_default()
    }

    /// Records brightness (`0x10` or `0x6B`) or contrast (`0x12`). Other codes are ignored.
    pub fn remember(&self, id: &str, code: u8, value: u32) {
        let slot = match code {
            vcp::BRIGHTNESS | vcp::BACKLIGHT_WHITE => Slot::Brightness,
            vcp::CONTRAST => Slot::Contrast,
            _ => return,
        };
        let mut map = self.inner.lock();
        let entry = map.entry(id.to_string()).or_default();
        let field = match slot {
            Slot::Brightness => &mut entry.brightness,
            Slot::Contrast => &mut entry.contrast,
        };
        if *field == Some(value) {
            return;
        }
        *field = Some(value);
        self.persist(&map);
    }

    fn persist(&self, map: &BTreeMap<String, Saved>) {
        let Some(path) = &self.path else { return };
        if let Err(e) = save(path, map) {
            tracing::warn!("saving {}: {e:#}", path.display());
        }
    }
}

enum Slot {
    Brightness,
    Contrast,
}

fn load(path: &Path) -> BTreeMap<String, Saved> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            tracing::warn!("reading {}: {e:#}", path.display());
            return BTreeMap::new();
        }
    };
    let file: FileModel = match toml::from_str(&text) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!("parsing {}: {e:#}", path.display());
            return BTreeMap::new();
        }
    };
    let mut map = BTreeMap::new();
    for row in file.monitor {
        if row.id.is_empty() {
            continue;
        }
        map.insert(
            row.id,
            Saved {
                brightness: row.brightness,
                contrast: row.contrast,
            },
        );
    }
    map
}

fn save(path: &Path, map: &BTreeMap<String, Saved>) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = FileModel {
        monitor: map
            .iter()
            .map(|(id, saved)| Row {
                id: id.clone(),
                brightness: saved.brightness,
                contrast: saved.contrast,
            })
            .collect(),
    };
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(&file)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_id_punctuation_and_one_field() {
        let dir = std::env::temp_dir().join(format!(
            "lumenctl-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.toml");
        let store = Store::open(Some(path.clone()));
        let id = "HWV6E22#5&26d47761&0&UID4355";
        store.remember(id, vcp::BRIGHTNESS, 40);
        store.remember(id, vcp::CONTRAST, 55);
        store.remember(id, vcp::SPEAKER_VOLUME, 10);
        store.remember(id, vcp::BACKLIGHT_WHITE, 41);

        let again = Store::open(Some(path));
        assert_eq!(
            again.get(id),
            Saved {
                brightness: Some(41),
                contrast: Some(55)
            }
        );
        assert_eq!(again.get("missing"), Saved::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
