//! OS-level operations that are not DDC/CI: system volume, display rotation, global power-off.

#[cfg(windows)]
mod imp {
    use std::mem::size_of;

    use anyhow::{bail, Context};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromPoint,
        CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYORIENTATION, DM_PELSHEIGHT,
        DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, SendMessageTimeoutW, HWND_BROADCAST, SC_MONITORPOWER, SMTO_ABORTIFHUNG,
        WM_SYSCOMMAND,
    };

    use crate::backend::windows::{from_wide, to_wide};

    pub fn monitor_under_cursor() -> Option<String> {
        unsafe {
            let mut pt = POINT::default();
            GetCursorPos(&mut pt).ok()?;
            let h = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
                return None;
            }
            Some(from_wide(&mi.szDevice))
        }
    }

    /// Puts every monitor into power-save the way Windows does on idle.
    pub fn all_monitors_off() {
        unsafe {
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SYSCOMMAND,
                WPARAM(SC_MONITORPOWER as usize),
                LPARAM(2),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
    }

    /// Rotates a display clockwise by 90 degrees.
    pub fn rotate_clockwise(gdi_device: &str) -> anyhow::Result<()> {
        let dev = to_wide(gdi_device);
        unsafe {
            let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
            if !EnumDisplaySettingsW(PCWSTR(dev.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                bail!("EnumDisplaySettings failed for {gdi_device}");
            }
            let orientation = dm.Anonymous1.Anonymous2.dmDisplayOrientation.0;
            // DMDO_DEFAULT=0, 90=1, 180=2, 270=3; Windows' 90 means counter-clockwise.
            let next = (orientation + 3) % 4;
            if (orientation % 2) != (next % 2) {
                std::mem::swap(&mut dm.dmPelsWidth, &mut dm.dmPelsHeight);
            }
            dm.Anonymous1.Anonymous2.dmDisplayOrientation.0 = next;
            dm.dmFields |= DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT;
            let r = ChangeDisplaySettingsExW(
                PCWSTR(dev.as_ptr()),
                Some(&dm),
                None,
                CDS_UPDATEREGISTRY,
                None,
            );
            if r != DISP_CHANGE_SUCCESSFUL {
                bail!("ChangeDisplaySettingsEx returned {r:?}");
            }
        }
        Ok(())
    }

    fn endpoint() -> anyhow::Result<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator")?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            Ok(device.Activate(CLSCTX_ALL, None)?)
        }
    }

    pub fn system_volume() -> anyhow::Result<(u32, bool)> {
        let ep = endpoint()?;
        unsafe {
            let v = ep.GetMasterVolumeLevelScalar()?;
            let muted = ep.GetMute()?.as_bool();
            Ok(((v * 100.0).round() as u32, muted))
        }
    }

    pub fn set_system_volume(percent: u32) -> anyhow::Result<()> {
        let ep = endpoint()?;
        unsafe { Ok(ep.SetMasterVolumeLevelScalar(percent.min(100) as f32 / 100.0, std::ptr::null())?) }
    }

    pub fn set_system_mute(mute: bool) -> anyhow::Result<()> {
        let ep = endpoint()?;
        unsafe { Ok(ep.SetMute(mute, std::ptr::null())?) }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn monitor_under_cursor() -> Option<String> {
        None
    }
    pub fn all_monitors_off() {}
    pub fn rotate_clockwise(_gdi_device: &str) -> anyhow::Result<()> {
        anyhow::bail!("not supported on this platform")
    }
    pub fn system_volume() -> anyhow::Result<(u32, bool)> {
        anyhow::bail!("not supported on this platform")
    }
    pub fn set_system_volume(_percent: u32) -> anyhow::Result<()> {
        anyhow::bail!("not supported on this platform")
    }
    pub fn set_system_mute(_mute: bool) -> anyhow::Result<()> {
        anyhow::bail!("not supported on this platform")
    }
}

pub use imp::*;
