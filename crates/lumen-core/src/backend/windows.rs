//! DDC/CI via dxva2 (Monitor Configuration API) plus display topology from GDI/CCD.

use std::collections::HashMap;
use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo,
    GetCapabilitiesStringLength, GetDisplayConfigBufferSizes,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, QueryDisplayConfig, SaveCurrentSettings, SetVCPFeature,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW,
    DISPLAY_DEVICE_ACTIVE, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
use windows::Win32::UI::WindowsAndMessaging::{EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY};

use super::{Backend, Device, Discovered};
use crate::edid;
use crate::model::{Connection, MonitorInfo, Rect, VcpValue};

pub struct WindowsBackend;

/// Normalizes the many return conventions windows-rs uses for BOOL-style APIs.
trait Succeeded {
    fn succeeded(self) -> bool;
}
impl Succeeded for i32 {
    fn succeeded(self) -> bool {
        self != 0
    }
}
impl Succeeded for BOOL {
    fn succeeded(self) -> bool {
        self.as_bool()
    }
}
impl Succeeded for windows::core::Result<()> {
    fn succeeded(self) -> bool {
        self.is_ok()
    }
}

struct TargetName {
    friendly: String,
    output: &'static str,
    internal: bool,
}

impl Backend for WindowsBackend {
    fn enumerate(&self) -> anyhow::Result<Vec<Discovered>> {
        let targets = query_targets().unwrap_or_default();
        let mut out = Vec::new();

        for hmon in enum_hmonitors() {
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if !unsafe { GetMonitorInfoW(hmon, &mut mi as *mut _ as *mut MONITORINFO) }.succeeded() {
                continue;
            }
            let gdi_device = from_wide(&mi.szDevice);
            let r = mi.monitorInfo.rcMonitor;
            let rect = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
            let primary = mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;

            let device_paths = monitor_device_paths(&mi.szDevice);
            let physical = physical_monitors(hmon);

            for (i, pm) in physical.into_iter().enumerate() {
                let path = device_paths.get(i).cloned().unwrap_or_default();
                let id = id_from_device_path(&path)
                    .unwrap_or_else(|| format!("{}#{}", gdi_device.trim_start_matches("\\\\.\\"), i));
                let target = targets.get(&path.to_ascii_lowercase());
                let edid = read_edid(&id).and_then(|d| edid::parse(&d));
                let description_buf = pm.szPhysicalMonitorDescription;
                let description = from_wide(&description_buf);

                let name = target
                    .map(|t| t.friendly.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| edid.as_ref().and_then(|e| e.name.clone().or(e.unspecified_text.clone())))
                    .unwrap_or_else(|| description.clone());

                let info = MonitorInfo {
                    key: crate::model::key_from_id(&id),
                    id,
                    index: 0,
                    name,
                    alias: None,
                    manufacturer_id: edid.as_ref().map(|e| e.manufacturer_id.clone()),
                    product_code: edid.as_ref().map(|e| format!("{:04X}", e.product_code)),
                    serial: edid.as_ref().and_then(|e| e.serial()),
                    year: edid.as_ref().map(|e| e.year),
                    week: edid.as_ref().map(|e| e.week),
                    gdi_device: gdi_device.clone(),
                    description,
                    rect,
                    primary,
                    connection: if target.is_some_and(|t| t.internal) {
                        Connection::Internal
                    } else {
                        Connection::External
                    },
                    output: target.map(|t| t.output.to_string()),
                };
                out.push(Discovered {
                    info,
                    device: Box::new(DdcDevice { handle: SendHandle(pm.hPhysicalMonitor) }),
                });
            }
        }
        for (i, d) in out.iter_mut().enumerate() {
            d.info.index = i + 1;
        }
        Ok(out)
    }
}

struct SendHandle(HANDLE);
// The handle is only ever used from the owning monitor worker thread.
unsafe impl Send for SendHandle {}

pub struct DdcDevice {
    handle: SendHandle,
}

impl Drop for DdcDevice {
    fn drop(&mut self) {
        let _ = unsafe { DestroyPhysicalMonitor(self.handle.0) };
    }
}

const RETRIES: usize = 3;
/// ERROR_GRAPHICS_DDCCI_VCP_NOT_SUPPORTED: a definitive answer, retrying is pointless.
const VCP_NOT_SUPPORTED: i32 = 0xC0262584u32 as i32;

impl Device for DdcDevice {
    fn get_vcp(&mut self, code: u8) -> anyhow::Result<VcpValue> {
        let mut err = std::io::Error::from_raw_os_error(0);
        for attempt in 0..RETRIES {
            let mut current = 0u32;
            let mut max = 0u32;
            let ok = unsafe {
                GetVCPFeatureAndVCPFeatureReply(self.handle.0, code, None, &mut current, Some(&mut max))
            }
            .succeeded();
            if ok {
                return Ok(VcpValue { current, max });
            }
            err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(VCP_NOT_SUPPORTED) {
                break;
            }
            if attempt + 1 < RETRIES {
                sleep(Duration::from_millis(40));
            }
        }
        bail!("getVCP 0x{code:02X} failed: {err}")
    }

    fn set_vcp(&mut self, code: u8, value: u32) -> anyhow::Result<()> {
        for attempt in 0..RETRIES {
            if unsafe { SetVCPFeature(self.handle.0, code, value) }.succeeded() {
                return Ok(());
            }
            if attempt + 1 < RETRIES {
                sleep(Duration::from_millis(40));
            }
        }
        bail!("setVCP 0x{code:02X}={value} failed: {}", std::io::Error::last_os_error())
    }

    fn capabilities(&mut self) -> anyhow::Result<String> {
        let mut len = 0u32;
        if !unsafe { GetCapabilitiesStringLength(self.handle.0, &mut len) }.succeeded() || len == 0 {
            bail!("monitor did not report a capability string");
        }
        let mut buf = vec![0u8; len as usize];
        if !unsafe { CapabilitiesRequestAndCapabilitiesReply(self.handle.0, &mut buf) }.succeeded() {
            bail!("capability request failed: {}", std::io::Error::last_os_error());
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    fn save_settings(&mut self) -> anyhow::Result<()> {
        if unsafe { SaveCurrentSettings(self.handle.0) }.succeeded() {
            Ok(())
        } else {
            Err(anyhow!("SaveCurrentSettings failed"))
        }
    }
}

fn enum_hmonitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let v = &mut *(data.0 as *mut Vec<HMONITOR>);
        v.push(h);
        BOOL(1)
    }
    let mut v: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

fn physical_monitors(hmon: HMONITOR) -> Vec<PHYSICAL_MONITOR> {
    let mut n = 0u32;
    if !unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut n) }.succeeded() || n == 0 {
        return Vec::new();
    }
    let mut v = vec![PHYSICAL_MONITOR::default(); n as usize];
    if !unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut v) }.succeeded() {
        return Vec::new();
    }
    v
}

/// Interface paths (`\\?\DISPLAY#HWV6E22#5&26d47761&0&UID4355#{...}`) of active monitors on an adapter output.
fn monitor_device_paths(adapter: &[u16; 32]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let mut dd = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        let ok = unsafe {
            EnumDisplayDevicesW(PCWSTR(adapter.as_ptr()), i, &mut dd, EDD_GET_DEVICE_INTERFACE_NAME)
        }
        .succeeded();
        if !ok {
            break;
        }
        if dd.StateFlags & DISPLAY_DEVICE_ACTIVE == DISPLAY_DEVICE_ACTIVE {
            out.push(from_wide(&dd.DeviceID));
        }
        i += 1;
    }
    out
}

pub fn id_from_device_path(path: &str) -> Option<String> {
    let mut parts = path.split('#');
    parts.next()?;
    let pnp = parts.next()?;
    let instance = parts.next()?;
    if pnp.is_empty() || instance.is_empty() {
        return None;
    }
    Some(format!("{pnp}#{instance}"))
}

fn query_targets() -> anyhow::Result<HashMap<String, TargetName>> {
    let mut n_paths = 0u32;
    let mut n_modes = 0u32;
    let err = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) };
    if err != ERROR_SUCCESS {
        bail!("GetDisplayConfigBufferSizes: {err:?}");
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
    let err = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            None,
        )
    };
    if err != ERROR_SUCCESS {
        bail!("QueryDisplayConfig: {err:?}");
    }
    paths.truncate(n_paths as usize);

    let mut map = HashMap::new();
    for p in &paths {
        let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        t.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        t.header.adapterId = p.targetInfo.adapterId;
        t.header.id = p.targetInfo.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut t.header) } != 0 {
            continue;
        }
        let (output, internal) = output_technology(t.outputTechnology.0);
        map.insert(
            from_wide(&t.monitorDevicePath).to_ascii_lowercase(),
            TargetName { friendly: from_wide(&t.monitorFriendlyDeviceName), output, internal },
        );
    }
    Ok(map)
}

fn output_technology(v: i32) -> (&'static str, bool) {
    match v {
        0 => ("VGA", false),
        1 => ("S-Video", false),
        2 => ("Composite", false),
        3 => ("Component", false),
        4 => ("DVI", false),
        5 => ("HDMI", false),
        6 => ("LVDS", true),
        8 => ("D-Jpn", false),
        9 => ("SDI", false),
        10 => ("DisplayPort", false),
        11 => ("eDP", true),
        12 => ("UDI", false),
        13 => ("UDI (embedded)", true),
        14 => ("SDTV", false),
        15 => ("Miracast", false),
        16 => ("Indirect wired", false),
        17 => ("Indirect virtual", false),
        18 => ("DisplayPort USB tunnel", false),
        x if x == 0x80000000u32 as i32 => ("Internal", true),
        _ => ("Other", false),
    }
}

fn read_edid(id: &str) -> Option<Vec<u8>> {
    let (pnp, instance) = id.split_once('#')?;
    let key = format!("SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{pnp}\\{instance}\\Device Parameters");
    let key_w = to_wide(&key);
    let value_w = to_wide("EDID");
    let mut buf = vec![0u8; 512];
    let mut len = buf.len() as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_w.as_ptr()),
            PCWSTR(value_w.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(len as usize);
    Some(buf)
}

pub(crate) fn from_wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_matches_clickmonitor_format() {
        assert_eq!(
            id_from_device_path(
                r"\\?\DISPLAY#HWV6E22#5&26d47761&0&UID4355#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
            )
            .as_deref(),
            Some("HWV6E22#5&26d47761&0&UID4355")
        );
    }
}
