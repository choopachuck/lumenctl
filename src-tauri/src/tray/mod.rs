//! Native notification-area icons: one numeric icon per monitor and feature.
//!
//! Tauri's tray API has no mouse-wheel events, so this runs its own Win32 window on a dedicated
//! thread. A low-level mouse hook catches wheel input over our icons; left click opens the GUI,
//! middle click toggles between two values, right click opens a menu.

mod icon;

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::thread;

use lumen_core::command::Value;
use lumen_core::model::PowerState;
use lumen_core::vcp::Feature;
use lumen_core::{platform, Engine, Event};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SendMessageTimeoutW,
    SetForegroundWindow, SetTimer, SetWindowsHookExW, TrackPopupMenuEx, TranslateMessage, HICON,
    HMENU, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, MSLLHOOKSTRUCT, SMTO_ABORTIFHUNG,
    SM_CXSMICON, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WH_MOUSE_LL, WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_DISPLAYCHANGE,
    WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPED,
};

pub struct Hooks {
    pub open: Box<dyn Fn() + Send>,
    pub quit: Box<dyn Fn() + Send>,
}

const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH: u32 = WM_APP + 2;
const WM_WHEEL: u32 = WM_APP + 3;
const WM_SHUTDOWN: u32 = WM_APP + 4;
const TIMER_RESCAN: usize = 1;
const TIMER_VOLUME: usize = 2;
/// Monitor uIDs never have both low bits set (see `stable_uid`), so this cannot collide.
const VOLUME_UID: u32 = 0xFFFF;

const CMD_OPEN: usize = 1;
const CMD_RESCAN: usize = 2;
const CMD_QUIT: usize = 3;
const CMD_MENU_BASE: usize = 100;
const CMD_POWER_BASE: usize = 1000;
const CMD_INPUT_BASE: usize = 2000;

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static REFRESH_PENDING: AtomicBool = AtomicBool::new(false);
/// Screen rectangles of our icons, read by the mouse hook without touching the shell.
static ICON_RECTS: Mutex<Vec<(RECT, u32)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Brightness,
    Contrast,
    Volume,
}

struct Entry {
    monitor_id: Option<String>,
    kind: Kind,
    text: String,
    tip: String,
    color: String,
    hicon: HICON,
}

struct State {
    engine: Engine,
    hooks: Hooks,
    hwnd: HWND,
    entries: HashMap<u32, Entry>,
    middle_toggle: HashMap<u32, bool>,
    taskbar_created: u32,
    menu_actions: HashMap<usize, MenuAction>,
}

enum MenuAction {
    Command(String),
    Power(String),
    Input(String, u32),
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn spawn(engine: Engine, hooks: Hooks) {
    thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            if let Err(e) = run(engine, hooks) {
                tracing::error!("tray: {e:#}");
            }
        })
        .expect("spawn tray thread");
}

fn run(engine: Engine, hooks: Hooks) -> anyhow::Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class = w!("lumenctl_tray");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("lumenctl"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        HWND_RAW.store(hwnd.0 as isize, Ordering::SeqCst);

        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                engine: engine.clone(),
                hooks,
                hwnd,
                entries: HashMap::new(),
                middle_toggle: HashMap::new(),
                taskbar_created,
                menu_actions: HashMap::new(),
            })
        });

        // Forward engine events to the window thread, coalescing bursts into one repaint.
        let mut rx = engine.subscribe();
        thread::Builder::new().name("tray-events".into()).spawn(move || loop {
            match rx.blocking_recv() {
                Ok(Event::Quit) | Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Ok(_) | Err(_) => request_refresh(),
            }
        })?;

        let _hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinstance.into()), 0)?;
        SetTimer(Some(hwnd), TIMER_VOLUME, 2000, None);
        request_refresh();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn request_refresh() {
    if !REFRESH_PENDING.swap(true, Ordering::SeqCst) {
        let hwnd = HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEWHEEL {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let pt = info.pt;
        let hit = ICON_RECTS.lock().ok().and_then(|rects| {
            rects
                .iter()
                .find(|(r, _)| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom)
                .map(|(_, uid)| *uid)
        });
        if let Some(uid) = hit {
            let delta = (info.mouseData >> 16) as i16;
            let hwnd = HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _);
            let _ = PostMessageW(Some(hwnd), WM_WHEEL, WPARAM(uid as usize), LPARAM(delta as isize));
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = STATE.with(|s| {
        let mut guard = match s.try_borrow_mut() {
            Ok(g) => g,
            Err(_) => {
                // Re-entered from the popup menu loop; the menu requests a refresh when it closes.
                if msg == WM_REFRESH {
                    REFRESH_PENDING.store(false, Ordering::SeqCst);
                }
                return false;
            }
        };
        let Some(state) = guard.as_mut() else { return false };
        match msg {
            WM_REFRESH => {
                REFRESH_PENDING.store(false, Ordering::SeqCst);
                state.sync_icons();
                true
            }
            WM_WHEEL => {
                state.on_wheel(wparam.0 as u32, lparam.0 as i16);
                true
            }
            WM_TRAY => {
                let event = (lparam.0 & 0xFFFF) as u32;
                let uid = ((lparam.0 >> 16) & 0xFFFF) as u32;
                match event {
                    WM_MOUSEMOVE => state.update_rects(),
                    WM_LBUTTONUP => (state.hooks.open)(),
                    WM_MBUTTONUP => state.on_middle(uid),
                    WM_CONTEXTMENU => {
                        // The menu loop re-enters wndproc; release the borrow first.
                        let actions = state.build_menu_and_track();
                        drop(guard);
                        if let Some(action) = actions {
                            run_menu_action(action);
                        }
                        request_refresh();
                        return true;
                    }
                    _ => {}
                }
                true
            }
            WM_TIMER if wparam.0 == TIMER_RESCAN => {
                let _ = KillTimer(Some(hwnd), TIMER_RESCAN);
                let engine = state.engine.clone();
                thread::spawn(move || {
                    let _ = engine.rescan();
                });
                true
            }
            WM_TIMER if wparam.0 == TIMER_VOLUME => {
                if state.engine.config().tray.show_system_volume_icon {
                    state.sync_icons();
                }
                true
            }
            WM_DISPLAYCHANGE => {
                // Monitors (dis)connected or woke up: rescan once things settle.
                SetTimer(Some(hwnd), TIMER_RESCAN, 3000, None);
                false
            }
            WM_SHUTDOWN => {
                state.remove_all();
                PostQuitMessage(0);
                true
            }
            m if m == state.taskbar_created && m != 0 => {
                // Explorer restarted and forgot our icons: re-add all of them.
                for (_, e) in state.entries.drain() {
                    let _ = DestroyIcon(e.hicon);
                }
                state.sync_icons();
                true
            }
            _ => false,
        }
    });
    if handled {
        LRESULT(0)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

enum PendingAction {
    Open,
    Quit,
    Rescan(Engine),
    Run(Engine, MenuAction),
}

fn run_menu_action(action: PendingAction) {
    match action {
        PendingAction::Open => STATE.with(|s| {
            if let Some(st) = s.borrow().as_ref() {
                (st.hooks.open)()
            }
        }),
        PendingAction::Quit => STATE.with(|s| {
            if let Some(st) = s.borrow().as_ref() {
                (st.hooks.quit)()
            }
        }),
        PendingAction::Rescan(engine) => {
            thread::spawn(move || {
                let _ = engine.rescan();
            });
        }
        PendingAction::Run(engine, action) => {
            thread::spawn(move || {
                let r = match action {
                    MenuAction::Command(cmd) => engine.execute(&cmd).map(drop),
                    MenuAction::Power(id) => engine
                        .monitor(&id)
                        .ok_or_else(|| anyhow::anyhow!("monitor gone"))
                        .and_then(|m| engine.toggle_power(&m).map(drop)),
                    MenuAction::Input(id, v) => engine
                        .monitor(&id)
                        .ok_or_else(|| anyhow::anyhow!("monitor gone"))
                        .and_then(|m| engine.set_input(&m, &v.to_string()).map(drop)),
                };
                if let Err(e) = r {
                    tracing::warn!("menu action: {e:#}");
                }
            });
        }
    }
}

fn wide_into<const N: usize>(dst: &mut [u16; N], s: &str) {
    let mut i = 0;
    for c in s.encode_utf16().take(N - 1) {
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

fn stable_uid(monitor_id: &str, kind: Kind) -> u32 {
    // FNV-1a: Windows remembers icon placement per (exe, uID), so uIDs must survive restarts.
    let mut h: u32 = 0x811C_9DC5;
    for b in monitor_id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    // Callback messages only carry the low 16 bits of the uID.
    let base = (h & 0x3FFF) << 2;
    base | kind as u32
}

impl State {
    fn nid(&self, uid: u32) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: uid,
            uCallbackMessage: WM_TRAY,
            ..Default::default()
        }
    }

    /// Computes the desired icon set from engine state and applies only the differences.
    fn sync_icons(&mut self) {
        let cfg = self.engine.config();
        if !cfg.tray.enabled {
            self.remove_all();
            return;
        }
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let mut desired: Vec<(u32, Option<String>, Kind, String, String, String, String)> = Vec::new();

        for m in self.engine.monitor_handles() {
            let snap = m.snapshot();
            let mc = cfg.monitor(&snap.info.id);
            let font = mc.and_then(|c| c.font.clone()).unwrap_or_else(|| cfg.tray.font.clone());
            let off = snap.power.is_some_and(|p| p != PowerState::On);
            let mut add = |kind: Kind, value: Option<lumen_core::VcpValue>, label: &str, color: &str| {
                let text = match value {
                    _ if off => "–".to_string(),
                    Some(v) => v.percent().to_string(),
                    None if snap.ddc_ok => "–".to_string(),
                    None => "?".to_string(),
                };
                let tip = format!(
                    "{} · {label} {}\nScroll: change · Middle click: toggle",
                    snap.display_name,
                    value.map(|v| format!("{}%", v.percent())).unwrap_or_else(|| "n/a".into())
                );
                desired.push((
                    stable_uid(&snap.info.id, kind),
                    Some(snap.info.id.clone()),
                    kind,
                    text,
                    tip,
                    color.to_string(),
                    font.clone(),
                ));
            };
            if mc.and_then(|c| c.show_brightness_icon).unwrap_or(true) {
                add(Kind::Brightness, snap.brightness, "Brightness", &cfg.tray.brightness_color);
            }
            if mc.and_then(|c| c.show_contrast_icon).unwrap_or(false) {
                add(Kind::Contrast, snap.contrast, "Contrast", &cfg.tray.contrast_color);
            }
        }
        if cfg.tray.show_system_volume_icon {
            if let Ok((vol, muted)) = platform::system_volume() {
                desired.push((
                    VOLUME_UID,
                    None,
                    Kind::Volume,
                    if muted { "M".into() } else { vol.to_string() },
                    format!("System volume {vol}%{}", if muted { " (muted)" } else { "" }),
                    cfg.tray.volume_color.clone(),
                    cfg.tray.font.clone(),
                ));
            }
        }

        let keep: Vec<u32> = desired.iter().map(|d| d.0).collect();
        let stale: Vec<u32> = self.entries.keys().copied().filter(|k| !keep.contains(k)).collect();
        for uid in stale {
            self.remove(uid);
        }

        for (uid, monitor_id, kind, text, tip, color, font) in desired {
            let unchanged = self
                .entries
                .get(&uid)
                .is_some_and(|e| e.text == text && e.tip == tip && e.color == color);
            if unchanged {
                continue;
            }
            let Some(hicon) = icon::render(&text, icon::parse_color(&color), &font, size) else {
                continue;
            };
            let mut nid = self.nid(uid);
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
            nid.hIcon = hicon;
            wide_into(&mut nid.szTip, &tip);
            unsafe {
                if let Some(old) = self.entries.get(&uid) {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
                    let _ = DestroyIcon(old.hicon);
                } else {
                    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
                    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                    let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
                }
            }
            self.entries.insert(uid, Entry { monitor_id, kind, text, tip, color, hicon });
        }
        self.update_rects();
    }

    fn remove(&mut self, uid: u32) {
        if let Some(e) = self.entries.remove(&uid) {
            let nid = self.nid(uid);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                let _ = DestroyIcon(e.hicon);
            }
        }
    }

    fn remove_all(&mut self) {
        let uids: Vec<u32> = self.entries.keys().copied().collect();
        for uid in uids {
            self.remove(uid);
        }
        self.update_rects();
    }

    fn update_rects(&self) {
        let mut rects = Vec::new();
        for &uid in self.entries.keys() {
            let id = NOTIFYICONIDENTIFIER {
                cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: self.hwnd,
                uID: uid,
                ..Default::default()
            };
            if let Ok(r) = unsafe { Shell_NotifyIconGetRect(&id) } {
                rects.push((r, uid));
            }
        }
        if let Ok(mut g) = ICON_RECTS.lock() {
            *g = rects;
        }
    }

    fn on_wheel(&mut self, uid: u32, delta: i16) {
        let Some(entry) = self.entries.get(&uid) else { return };
        let cfg = self.engine.config();
        let notches = (delta as i32 / 120).max(-10).min(10);
        let notches = if notches == 0 { delta.signum() as i32 } else { notches };
        let (kind, monitor_id) = (entry.kind, entry.monitor_id.clone());
        let engine = self.engine.clone();
        match kind {
            Kind::Volume => {
                let step = cfg.tray.wheel_step_volume as i32 * notches;
                thread::spawn(move || {
                    let _ = engine.set_system_volume(Value::Rel(step));
                });
            }
            Kind::Brightness | Kind::Contrast => {
                let (feature, step) = if kind == Kind::Brightness {
                    (Feature::Brightness, cfg.tray.wheel_step_brightness)
                } else {
                    (Feature::Contrast, cfg.tray.wheel_step_contrast)
                };
                let Some(m) = monitor_id.and_then(|id| engine.monitor(&id)) else { return };
                let value = Value::Rel(step as i32 * notches);
                if m.cached(m.code(feature)).is_some() {
                    // Fast path: optimistic update, write is queued on the monitor thread.
                    let _ = engine.adjust(&m, feature, value);
                } else {
                    thread::spawn(move || {
                        let _ = engine.adjust(&m, feature, value);
                    });
                }
            }
        }
    }

    fn on_middle(&mut self, uid: u32) {
        let Some(entry) = self.entries.get(&uid) else { return };
        let cfg = self.engine.config();
        let flag = self.middle_toggle.entry(uid).or_insert(false);
        *flag = !*flag;
        let idx = if *flag { 0 } else { 1 };
        let engine = self.engine.clone();
        match entry.kind {
            Kind::Volume => {
                thread::spawn(move || {
                    let muted = platform::system_volume().map(|v| v.1).unwrap_or(false);
                    let _ = engine.set_system_mute(!muted);
                });
            }
            kind => {
                let (feature, values) = if kind == Kind::Brightness {
                    (Feature::Brightness, cfg.tray.middle_click_brightness)
                } else {
                    (Feature::Contrast, cfg.tray.middle_click_contrast)
                };
                let Some(m) = entry.monitor_id.as_ref().and_then(|id| engine.monitor(id)) else { return };
                thread::spawn(move || {
                    let _ = engine.adjust(&m, feature, Value::Abs(values[idx]));
                });
            }
        }
    }

    fn build_menu_and_track(&mut self) -> Option<PendingAction> {
        let cfg = self.engine.config();
        self.menu_actions.clear();
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let add = |m: HMENU, id: usize, text: &str| {
                let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(m, MF_STRING, id, PCWSTR(w.as_ptr()));
            };
            add(menu, CMD_OPEN, "Open lumenctl");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);

            for (i, entry) in cfg.menu.iter().enumerate() {
                add(menu, CMD_MENU_BASE + i, &entry.label);
                self.menu_actions.insert(CMD_MENU_BASE + i, MenuAction::Command(entry.command.clone()));
            }
            if !cfg.menu.is_empty() {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            }

            let mut submenus = Vec::new();
            for (i, m) in self.engine.monitor_handles().iter().enumerate() {
                let snap = m.snapshot();
                let sub = CreatePopupMenu().ok()?;
                let power_id = CMD_POWER_BASE + i;
                let on = snap.power.map_or(true, |p| p == PowerState::On);
                add(sub, power_id, if on { "Power off" } else { "Power on" });
                self.menu_actions.insert(power_id, MenuAction::Power(snap.info.id.clone()));
                let _ = AppendMenuW(sub, MF_SEPARATOR, 0, None);
                let header: Vec<u16> = "Input".encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, PCWSTR(header.as_ptr()));
                for (j, input) in snap.supported_inputs.iter().enumerate() {
                    let id = CMD_INPUT_BASE + i * 64 + j;
                    let current = snap.input.as_ref().is_some_and(|c| c.value == input.value);
                    let label = if current { format!("● {}", input.name) } else { format!("   {}", input.name) };
                    add(sub, id, &label);
                    self.menu_actions.insert(id, MenuAction::Input(snap.info.id.clone(), input.value));
                }
                let title: Vec<u16> = format!("{}. {}", snap.info.index, snap.display_name)
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(title.as_ptr()));
                submenus.push(sub);
            }
            if !submenus.is_empty() {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            }
            add(menu, CMD_RESCAN, "Rescan monitors");
            add(menu, CMD_QUIT, "Quit");

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
                pt.x,
                pt.y,
                self.hwnd,
                None,
            )
            .0 as usize;
            let _ = DestroyMenu(menu);

            match cmd {
                0 => None,
                CMD_OPEN => Some(PendingAction::Open),
                CMD_QUIT => Some(PendingAction::Quit),
                CMD_RESCAN => Some(PendingAction::Rescan(self.engine.clone())),
                id => self
                    .menu_actions
                    .remove(&id)
                    .map(|a| PendingAction::Run(self.engine.clone(), a)),
            }
        }
    }
}

/// Removes all icons and stops the tray thread; blocks until done so no ghost icons remain.
pub fn shutdown() {
    let hwnd = HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _);
    if hwnd.0.is_null() {
        return;
    }
    unsafe {
        let _ = SendMessageTimeoutW(hwnd, WM_SHUTDOWN, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 2000, None);
    }
}
