//! Tauri commands used by the GUI. Blocking engine calls run on the blocking thread pool.

use std::sync::Arc;

use lumen_core::caps::Capabilities;
use lumen_core::engine::Monitor;
use lumen_core::patch::{MonitorPatch, ValueSpec};
use lumen_core::{platform, Config, Engine, ExecOutcome, MonitorSnapshot, VcpValue};
use serde::Serialize;
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

type CmdResult<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

fn monitor(engine: &Engine, reference: &str) -> CmdResult<Arc<Monitor>> {
    engine.resolve(reference).ok_or_else(|| format!("monitor '{reference}' not found"))
}

#[tauri::command]
pub fn get_monitors(engine: State<'_, Engine>) -> Vec<MonitorSnapshot> {
    engine.monitors()
}

#[tauri::command]
pub async fn patch_monitor(engine: State<'_, Engine>, reference: String, patch: MonitorPatch) -> CmdResult<MonitorSnapshot> {
    let engine = engine.inner().clone();
    let m = monitor(&engine, &reference)?;
    blocking(move || {
        engine.apply_patch(&m, &patch)?;
        Ok(m.snapshot())
    })
    .await
}

#[tauri::command]
pub async fn execute_command(engine: State<'_, Engine>, command: String) -> CmdResult<ExecOutcome> {
    let engine = engine.inner().clone();
    blocking(move || engine.execute(&command)).await
}

#[tauri::command]
pub async fn rescan(engine: State<'_, Engine>) -> CmdResult<Vec<MonitorSnapshot>> {
    let engine = engine.inner().clone();
    blocking(move || {
        engine.rescan()?;
        Ok(engine.monitors())
    })
    .await
}

#[tauri::command]
pub fn refresh_monitor(engine: State<'_, Engine>, reference: String) -> CmdResult<()> {
    monitor(&engine, &reference)?.refresh();
    Ok(())
}

#[tauri::command]
pub async fn get_capabilities(engine: State<'_, Engine>, reference: String, refresh: bool) -> CmdResult<Capabilities> {
    let m = monitor(&engine, &reference)?;
    blocking(move || m.capabilities(refresh)).await
}

#[tauri::command]
pub async fn read_vcp(engine: State<'_, Engine>, reference: String, code: u8) -> CmdResult<VcpValue> {
    let m = monitor(&engine, &reference)?;
    blocking(move || m.read(code)).await
}

#[tauri::command]
pub async fn write_vcp(engine: State<'_, Engine>, reference: String, code: u8, value: ValueSpec) -> CmdResult<u32> {
    let engine = engine.inner().clone();
    let m = monitor(&engine, &reference)?;
    blocking(move || engine.set_vcp(&m, code, value.0)).await
}

#[tauri::command]
pub async fn monitor_action(engine: State<'_, Engine>, reference: String, action: String) -> CmdResult<()> {
    let engine = engine.inner().clone();
    let m = monitor(&engine, &reference)?;
    blocking(move || {
        match action.as_str() {
            "factory_reset" => engine.factory_reset(&m),
            "rotate" => platform::rotate_clockwise(&m.info().gdi_device)?,
            "power_toggle" => {
                engine.toggle_power(&m)?;
            }
            other => anyhow::bail!("unknown action '{other}'"),
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn select_monitor(engine: State<'_, Engine>, id: Option<String>) {
    engine.select(id);
}

#[tauri::command]
pub fn get_config(engine: State<'_, Engine>) -> Config {
    engine.config()
}

#[tauri::command]
pub fn save_config(app: tauri::AppHandle, engine: State<'_, Engine>, config: Config) -> CmdResult<()> {
    let autostart = config.general.start_with_windows;
    engine.update_config(|c| *c = config).map_err(|e| format!("{e:#}"))?;
    let launcher = app.autolaunch();
    let r = if autostart { launcher.enable() } else { launcher.disable() };
    r.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct SystemVolume {
    volume: u32,
    muted: bool,
}

#[tauri::command]
pub async fn get_system_volume() -> CmdResult<SystemVolume> {
    blocking(|| platform::system_volume().map(|(volume, muted)| SystemVolume { volume, muted })).await
}

#[tauri::command]
pub async fn set_system_volume(engine: State<'_, Engine>, volume: Option<ValueSpec>, muted: Option<bool>) -> CmdResult<()> {
    let engine = engine.inner().clone();
    blocking(move || {
        if let Some(v) = volume {
            engine.set_system_volume(v.0)?;
        }
        if let Some(m) = muted {
            engine.set_system_mute(m)?;
        }
        Ok(())
    })
    .await
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    config_path: String,
    mqtt_node_id: String,
}

#[tauri::command]
pub fn app_info(engine: State<'_, Engine>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        config_path: Config::default_path().display().to_string(),
        mqtt_node_id: lumen_server::mqtt::node_id(&engine.config().mqtt),
    }
}

/// Installed font families, for the tray icon font pickers.
#[tauri::command]
pub fn list_fonts() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::collections::BTreeSet;
        use windows::Win32::Foundation::LPARAM;
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, EnumFontFamiliesExW, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW,
        };

        unsafe extern "system" fn callback(
            lf: *const LOGFONTW,
            _: *const TEXTMETRICW,
            _: u32,
            data: LPARAM,
        ) -> i32 {
            let set = &mut *(data.0 as *mut BTreeSet<String>);
            let face = unsafe { (*lf).lfFaceName };
            let end = face.iter().position(|&c| c == 0).unwrap_or(face.len());
            let name = String::from_utf16_lossy(&face[..end]);
            if !name.is_empty() && !name.starts_with('@') {
                set.insert(name);
            }
            1
        }

        let mut set = BTreeSet::new();
        unsafe {
            let hdc = CreateCompatibleDC(None);
            let mut lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
            EnumFontFamiliesExW(hdc, &mut lf, Some(callback), LPARAM(&mut set as *mut _ as isize), 0);
            let _ = DeleteDC(hdc);
        }
        return set.into_iter().collect();
    }
    #[cfg(not(windows))]
    Vec::new()
}
