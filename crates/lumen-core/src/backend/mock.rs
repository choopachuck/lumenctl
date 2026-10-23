use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::{Backend, Device, Discovered};
use crate::model::{MonitorInfo, VcpValue};
use crate::vcp;

/// In-memory monitors, used by tests and on non-Windows hosts.
pub struct MockBackend {
    pub monitors: Vec<(MonitorInfo, Arc<Mutex<BTreeMap<u8, VcpValue>>>)>,
}

impl MockBackend {
    pub fn with_monitors(n: usize) -> Self {
        let monitors = (0..n)
            .map(|i| {
                let info = MonitorInfo {
                    id: format!("MOCK{i:04}#1&0&UID{i}"),
                    index: i + 1,
                    name: format!("Mock{}", i + 1),
                    serial: Some(format!("SN{i}")),
                    gdi_device: format!("\\\\.\\DISPLAY{}", i + 1),
                    description: "Mock monitor".into(),
                    ..Default::default()
                };
                let mut values = BTreeMap::new();
                for (code, cur, max) in [
                    (vcp::BRIGHTNESS, 50, 100),
                    (vcp::CONTRAST, 50, 100),
                    (vcp::RED_GAIN, 100, 100),
                    (vcp::GREEN_GAIN, 100, 100),
                    (vcp::BLUE_GAIN, 100, 100),
                    (vcp::SPEAKER_VOLUME, 30, 100),
                    (vcp::INPUT_SOURCE, 0x0F, 0xFF),
                    (vcp::COLOR_PRESET, 0x05, 0x0D),
                    (vcp::DISPLAY_MODE, 0x00, 0xFF),
                    (vcp::POWER_MODE, 0x01, 0x05),
                ] {
                    values.insert(code, VcpValue { current: cur, max });
                }
                (info, Arc::new(Mutex::new(values)))
            })
            .collect();
        Self { monitors }
    }
}

impl Backend for MockBackend {
    fn enumerate(&self) -> anyhow::Result<Vec<Discovered>> {
        Ok(self
            .monitors
            .iter()
            .map(|(info, values)| Discovered {
                info: info.clone(),
                device: Box::new(MockDevice { values: values.clone() }),
            })
            .collect())
    }
}

pub struct MockDevice {
    values: Arc<Mutex<BTreeMap<u8, VcpValue>>>,
}

impl Device for MockDevice {
    fn get_vcp(&mut self, code: u8) -> anyhow::Result<VcpValue> {
        self.values
            .lock()
            .get(&code)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("VCP 0x{code:02X} not supported"))
    }

    fn set_vcp(&mut self, code: u8, value: u32) -> anyhow::Result<()> {
        let mut values = self.values.lock();
        let entry = values.entry(code).or_insert(VcpValue { current: 0, max: 100 });
        entry.current = value;
        Ok(())
    }

    fn capabilities(&mut self) -> anyhow::Result<String> {
        Ok("(prot(monitor)type(LCD)model(Mock)vcp(04 10 12 14(05 08 0B) 16 18 1A 60(0F 11 12) 62 D6(01 04 05) DC(00 03 05))mccs_ver(2.2))".into())
    }

    fn save_settings(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
