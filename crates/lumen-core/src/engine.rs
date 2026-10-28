use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::backend::{Backend, Device};
use crate::caps::{self, Capabilities};
use crate::command::{self, Program, Step, Value};
use crate::config::Config;
use crate::model::{MonitorInfo, MonitorSnapshot, MonitorState, PowerState, VcpValue};
use crate::platform;
use crate::state::Store;
use crate::vcp::{self, Feature};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    MonitorsChanged { monitors: Vec<MonitorSnapshot> },
    MonitorUpdated { monitor: MonitorSnapshot },
    SystemVolume { volume: u32, muted: bool },
    Notification { message: String },
    ConfigChanged,
    Quit,
    Restart,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExecOutcome {
    pub messages: Vec<String>,
    /// Value requested by `d`, `e` or `get_VCP` (ClickMonitorDDC returned it as errorlevel).
    pub value: Option<i64>,
    pub quit: bool,
    pub restart: bool,
}

enum Job {
    Write {
        code: u8,
        value: u32,
    },
    Read {
        code: u8,
        reply: Option<mpsc::Sender<anyhow::Result<VcpValue>>>,
    },
    Capabilities {
        reply: Option<mpsc::Sender<anyhow::Result<Capabilities>>>,
    },
    Poll,
    /// Write saved brightness and contrast when the monitor currently differs.
    ApplySaved {
        reply: mpsc::Sender<bool>,
    },
    Shutdown,
}

pub struct Monitor {
    info: RwLock<MonitorInfo>,
    state: RwLock<MonitorState>,
    use_6b: AtomicBool,
    verify: AtomicBool,
    save_nvram: AtomicBool,
    tx: mpsc::Sender<Job>,
    events: broadcast::Sender<Event>,
    store: Arc<Store>,
}

impl Monitor {
    pub fn id(&self) -> String {
        self.info.read().id.clone()
    }

    pub fn info(&self) -> MonitorInfo {
        self.info.read().clone()
    }

    pub fn snapshot(&self) -> MonitorSnapshot {
        MonitorSnapshot::build(
            &self.info.read(),
            &self.state.read(),
            self.use_6b.load(Ordering::Relaxed),
        )
    }

    pub fn code(&self, feature: Feature) -> u8 {
        feature.code(self.use_6b.load(Ordering::Relaxed))
    }

    pub fn cached(&self, code: u8) -> Option<VcpValue> {
        self.state.read().get(code)
    }

    fn emit(&self) {
        let _ = self.events.send(Event::MonitorUpdated {
            monitor: self.snapshot(),
        });
    }

    fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// Blocking read through the worker thread.
    pub fn read(&self, code: u8) -> anyhow::Result<VcpValue> {
        let (tx, rx) = mpsc::channel();
        self.send(Job::Read {
            code,
            reply: Some(tx),
        });
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("monitor did not answer"))?
    }

    /// Returns the cached value or reads it from the monitor.
    pub fn current(&self, code: u8) -> anyhow::Result<VcpValue> {
        match self.cached(code) {
            Some(v) => Ok(v),
            None => self.read(code),
        }
    }

    /// Updates the cached value immediately and queues the hardware write.
    pub fn write(&self, code: u8, value: u32) {
        {
            let mut st = self.state.write();
            let max = st.get(code).map(|v| v.max).unwrap_or(100);
            st.vcp.insert(
                code,
                VcpValue {
                    current: value,
                    max,
                },
            );
        }
        self.emit();
        self.send(Job::Write { code, value });
    }

    pub fn refresh(&self) {
        self.send(Job::Poll);
    }

    /// Blocking restore of saved brightness and contrast. `false` when the monitor did not answer.
    fn apply_saved(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        self.send(Job::ApplySaved { reply: tx });
        rx.recv_timeout(Duration::from_secs(20)).unwrap_or(false)
    }

    pub fn capabilities(&self, refresh: bool) -> anyhow::Result<Capabilities> {
        if !refresh {
            if let Some(c) = self.state.read().capabilities.clone() {
                return Ok(c);
            }
        }
        let (tx, rx) = mpsc::channel();
        self.send(Job::Capabilities { reply: Some(tx) });
        rx.recv_timeout(Duration::from_secs(15))
            .map_err(|_| anyhow!("monitor did not answer"))?
    }
}

struct Worker {
    monitor: Arc<Monitor>,
    device: Box<dyn Device>,
    /// Last value known to be stored in the monitor; identical writes are skipped.
    written: HashMap<u8, u32>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Job>) {
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            while let Ok(j) = rx.try_recv() {
                batch.push(j);
            }
            // Only the newest write per code matters (fast mouse-wheel scrolling).
            let mut last_write: HashMap<u8, usize> = HashMap::new();
            for (i, j) in batch.iter().enumerate() {
                if let Job::Write { code, .. } = j {
                    last_write.insert(*code, i);
                }
            }
            for (i, job) in batch.into_iter().enumerate() {
                match job {
                    Job::Write { code, value } if last_write.get(&code) == Some(&i) => {
                        self.write(code, value)
                    }
                    Job::Write { .. } => {}
                    Job::Read { code, reply } => {
                        let r = self.read(code);
                        self.monitor.emit();
                        if let Some(tx) = reply {
                            let _ = tx.send(r);
                        }
                    }
                    Job::Capabilities { reply } => {
                        let r = self.device.capabilities().map(|raw| caps::parse(&raw));
                        if let Ok(c) = &r {
                            self.monitor.state.write().capabilities = Some(c.clone());
                            self.monitor.emit();
                        }
                        if let Some(tx) = reply {
                            let _ = tx.send(r);
                        }
                    }
                    Job::Poll => self.poll(),
                    Job::ApplySaved { reply } => {
                        let ok = self.apply_saved();
                        self.monitor.emit();
                        let _ = reply.send(ok);
                    }
                    Job::Shutdown => return,
                }
            }
        }
    }

    fn read(&mut self, code: u8) -> anyhow::Result<VcpValue> {
        let r = self.device.get_vcp(code);
        let mut st = self.monitor.state.write();
        match &r {
            Ok(v) => {
                st.vcp.insert(code, *v);
                st.ddc_ok = true;
                self.written.insert(code, v.current);
            }
            Err(e) => st.last_error = Some(e.to_string()),
        }
        r
    }

    fn write(&mut self, code: u8, value: u32) {
        if self.written.get(&code) == Some(&value) {
            return;
        }
        match self.device.set_vcp(code, value) {
            Ok(()) => {
                self.written.insert(code, value);
                self.monitor.store.remember(&self.monitor.id(), code, value);
                if self.monitor.save_nvram.load(Ordering::Relaxed) {
                    let _ = self.device.save_settings();
                }
                if self.monitor.verify.load(Ordering::Relaxed) {
                    let _ = self.read(code);
                    self.monitor.emit();
                }
            }
            Err(e) => {
                warn!("{}: {e:#}", self.monitor.info.read().name);
                self.monitor.state.write().last_error = Some(e.to_string());
                // Resync the optimistic cache with what the monitor really has.
                let _ = self.read(code);
                self.monitor.emit();
            }
        }
    }

    fn poll(&mut self) {
        let use_6b = self.monitor.use_6b.load(Ordering::Relaxed);
        let mut any = false;
        for &code in vcp::POLLED {
            let code = if code == vcp::BRIGHTNESS && use_6b {
                vcp::BACKLIGHT_WHITE
            } else {
                code
            };
            any |= self.read(code).is_ok();
        }
        {
            let mut st = self.monitor.state.write();
            st.ddc_ok = any;
            if any {
                st.last_error = None;
            }
        }
        self.monitor.emit();
    }

    /// Puts back saved brightness and contrast. A missing value is recorded from the monitor
    /// and not written. `false` when brightness cannot be read (the monitor is not ready).
    fn apply_saved(&mut self) -> bool {
        let id = self.monitor.id();
        let saved = self.monitor.store.get(&id);
        let brightness = self.monitor.code(Feature::Brightness);
        if !self.align(&id, brightness, saved.brightness) {
            return false;
        }
        // No saved contrast yet: a monitor without VCP 0x12 must not fail the restore.
        self.align(&id, vcp::CONTRAST, saved.contrast) || saved.contrast.is_none()
    }

    fn align(&mut self, id: &str, code: u8, saved: Option<u32>) -> bool {
        match self.read(code) {
            Ok(v) if saved == Some(v.current) => true,
            Ok(v) => match saved {
                Some(want) => {
                    self.write(code, want);
                    self.written.get(&code) == Some(&want)
                }
                None => {
                    self.monitor.store.remember(id, code, v.current);
                    true
                }
            },
            Err(_) => false,
        }
    }
}

struct Inner {
    backend: Box<dyn Backend>,
    monitors: RwLock<Vec<Arc<Monitor>>>,
    config: RwLock<Config>,
    config_path: Option<PathBuf>,
    events: broadcast::Sender<Event>,
    toggles: Mutex<HashMap<String, usize>>,
    selected: RwLock<Option<String>>,
    rescan_lock: Mutex<()>,
    store: Arc<Store>,
    restore_lock: Mutex<()>,
}

#[derive(Clone)]
pub struct Engine(Arc<Inner>);

impl Engine {
    pub fn new(config: Config, config_path: Option<PathBuf>, backend: Box<dyn Backend>) -> Self {
        let (events, _) = broadcast::channel(256);
        let store = Arc::new(Store::open(
            config_path.as_ref().map(|p| p.with_file_name("state.toml")),
        ));
        Engine(Arc::new(Inner {
            backend,
            monitors: RwLock::new(Vec::new()),
            config: RwLock::new(config),
            config_path,
            events,
            toggles: Mutex::new(HashMap::new()),
            selected: RwLock::new(None),
            rescan_lock: Mutex::new(()),
            store,
            restore_lock: Mutex::new(()),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    pub fn emit(&self, event: Event) {
        let _ = self.0.events.send(event);
    }

    pub fn config(&self) -> Config {
        self.0.config.read().clone()
    }

    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> anyhow::Result<()> {
        let was_restore = self.config().general.restore_values;
        let snapshot = {
            let mut cfg = self.0.config.write();
            f(&mut cfg);
            cfg.clone()
        };
        if let Some(path) = &self.0.config_path {
            snapshot
                .save(path)
                .with_context(|| format!("saving {}", path.display()))?;
        }
        self.apply_config();
        if snapshot.general.restore_values && !was_restore {
            self.snapshot_current();
        }
        self.emit(Event::ConfigChanged);
        Ok(())
    }

    /// Re-applies saved brightness and contrast, retrying while monitors wake up.
    /// Does nothing when `general.restore_values` is off.
    pub fn restore_saved(&self) {
        self.restore_with(4, Duration::from_secs(2));
    }

    fn restore_with(&self, attempts: u32, pause: Duration) {
        if !self.config().general.restore_values {
            return;
        }
        let _guard = self.0.restore_lock.lock();
        if !self.config().general.restore_values {
            return;
        }
        let mut restored = false;
        for attempt in 0..attempts {
            if attempt > 0 {
                thread::sleep(pause);
            }
            if !self.config().general.restore_values {
                return;
            }
            let monitors = self.monitor_handles();
            if monitors.is_empty() {
                break;
            }
            if monitors.iter().all(|m| m.apply_saved()) {
                restored = true;
                break;
            }
            warn!("restore attempt {}/{} incomplete", attempt + 1, attempts);
        }
        if restored {
            info!("restored brightness and contrast");
        }
    }

    /// Copies the values currently shown in the UI into the saved state.
    fn snapshot_current(&self) {
        for m in self.monitor_handles() {
            let id = m.id();
            let brightness = m.code(Feature::Brightness);
            if let Some(v) = m.cached(brightness) {
                self.0.store.remember(&id, brightness, v.current);
            }
            if let Some(v) = m.cached(vcp::CONTRAST) {
                self.0.store.remember(&id, vcp::CONTRAST, v.current);
            }
        }
    }

    fn apply_config(&self) {
        let cfg = self.0.config.read();
        for m in self.0.monitors.read().iter() {
            let mut info = m.info.write();
            let mc = cfg.monitor(&info.id);
            info.alias = mc.and_then(|c| c.alias.clone()).filter(|a| !a.is_empty());
            m.use_6b
                .store(mc.is_some_and(|c| c.use_6b), Ordering::Relaxed);
            m.verify.store(cfg.general.verify_writes, Ordering::Relaxed);
            m.save_nvram
                .store(cfg.general.save_to_nvram, Ordering::Relaxed);
        }
        drop(cfg);
        self.emit_monitors();
    }

    /// Discovers monitors (dropping previous handles) and starts polling them in the background.
    pub fn rescan(&self) -> anyhow::Result<()> {
        let _guard = self.0.rescan_lock.lock();
        for m in self.0.monitors.write().drain(..) {
            m.send(Job::Shutdown);
        }
        let discovered = self.0.backend.enumerate()?;
        let cfg = self.config();
        let mut monitors = Vec::new();
        for d in discovered {
            let mc = cfg.monitor(&d.info.id);
            if mc.is_some_and(|c| c.ignore) {
                continue;
            }
            let mut info = d.info;
            info.key = crate::model::key_from_id(&info.id);
            info.alias = mc.and_then(|c| c.alias.clone()).filter(|a| !a.is_empty());
            let (tx, rx) = mpsc::channel();
            let monitor = Arc::new(Monitor {
                info: RwLock::new(info),
                state: RwLock::new(MonitorState::default()),
                use_6b: AtomicBool::new(mc.is_some_and(|c| c.use_6b)),
                verify: AtomicBool::new(cfg.general.verify_writes),
                save_nvram: AtomicBool::new(cfg.general.save_to_nvram),
                tx,
                events: self.0.events.clone(),
                store: self.0.store.clone(),
            });
            let worker = Worker {
                monitor: monitor.clone(),
                device: d.device,
                written: HashMap::new(),
            };
            let name = format!("ddc-{}", monitor.info.read().index);
            thread::Builder::new()
                .name(name)
                .spawn(move || worker.run(rx))?;
            monitor.send(Job::Poll);
            monitor.send(Job::Capabilities { reply: None });
            monitors.push(monitor);
        }
        info!("found {} monitor(s)", monitors.len());
        *self.0.monitors.write() = monitors;
        self.emit_monitors();
        Ok(())
    }

    fn emit_monitors(&self) {
        self.emit(Event::MonitorsChanged {
            monitors: self.monitors(),
        });
    }

    pub fn monitors(&self) -> Vec<MonitorSnapshot> {
        self.0
            .monitors
            .read()
            .iter()
            .map(|m| m.snapshot())
            .collect()
    }

    pub fn monitor_handles(&self) -> Vec<Arc<Monitor>> {
        self.0.monitors.read().clone()
    }

    pub fn monitor(&self, id: &str) -> Option<Arc<Monitor>> {
        self.0
            .monitors
            .read()
            .iter()
            .find(|m| m.info.read().id == id)
            .cloned()
    }

    /// Resolves a selector the way ClickMonitorDDC does: number, then exact name/alias/serial/id,
    /// then prefix (at least two characters). Spaces are ignored. All matches are returned.
    pub fn find(&self, selector: &str) -> Vec<Arc<Monitor>> {
        let monitors = self.0.monitors.read();
        if let Ok(n) = selector.trim().parse::<usize>() {
            return monitors
                .iter()
                .filter(|m| m.info.read().index == n)
                .cloned()
                .collect();
        }
        let norm = |s: &str| -> String {
            s.chars()
                .filter(|c| !c.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect()
        };
        let want = norm(selector);
        if want.len() < 2 {
            return Vec::new();
        }
        let fields = |i: &MonitorInfo| -> Vec<String> {
            [
                Some(&i.id),
                i.alias.as_ref(),
                Some(&i.name),
                i.serial.as_ref(),
                Some(&i.description),
            ]
            .into_iter()
            .flatten()
            .map(|s| norm(s))
            .collect()
        };
        let exact: Vec<_> = monitors
            .iter()
            .filter(|m| fields(&m.info.read()).iter().any(|f| *f == want))
            .cloned()
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        monitors
            .iter()
            .filter(|m| fields(&m.info.read()).iter().any(|f| f.starts_with(&want)))
            .cloned()
            .collect()
    }

    /// Single monitor by id, key, number, or the first name/alias match.
    pub fn resolve(&self, reference: &str) -> Option<Arc<Monitor>> {
        let by_id = self
            .0
            .monitors
            .read()
            .iter()
            .find(|m| {
                let i = m.info.read();
                i.id == reference || i.key == reference
            })
            .cloned();
        by_id.or_else(|| self.find(reference).into_iter().next())
    }

    pub fn select(&self, id: Option<String>) {
        *self.0.selected.write() = id;
    }

    /// Monitors targeted by commands without an explicit selector.
    pub fn default_targets(&self) -> Vec<Arc<Monitor>> {
        let cfg = self.config();
        let monitors = self.monitor_handles();
        if cfg.general.target_monitor_under_cursor {
            if let Some(dev) = platform::monitor_under_cursor() {
                let v: Vec<_> = monitors
                    .iter()
                    .filter(|m| m.info.read().gdi_device == dev)
                    .cloned()
                    .collect();
                if !v.is_empty() {
                    return v;
                }
            }
        }
        let selected = self
            .0
            .selected
            .read()
            .clone()
            .or(cfg.general.default_monitor);
        if let Some(sel) = selected {
            let v = self
                .monitor(&sel)
                .map(|m| vec![m])
                .unwrap_or_else(|| self.find(&sel));
            if !v.is_empty() {
                return v;
            }
        }
        monitors.into_iter().take(1).collect()
    }

    fn limits(&self, m: &Monitor, feature: Feature, max: u32) -> (u32, u32) {
        let cfg = self.0.config.read();
        let mc = cfg.monitor(&m.info.read().id);
        let max = if max == 0 { 100 } else { max };
        let (lo, hi) = match feature {
            Feature::Brightness => (
                mc.and_then(|c| c.min_brightness),
                mc.and_then(|c| c.max_brightness),
            ),
            Feature::Contrast => (
                mc.and_then(|c| c.min_contrast),
                mc.and_then(|c| c.max_contrast),
            ),
            _ => (None, None),
        };
        (lo.unwrap_or(0).min(max), hi.unwrap_or(max).min(max))
    }

    /// Changes a continuous feature; returns the new value.
    pub fn adjust(&self, m: &Monitor, feature: Feature, value: Value) -> anyhow::Result<u32> {
        let code = m.code(feature);
        let cur = m.current(code)?;
        let (lo, hi) = self.limits(m, feature, cur.max);
        let new = value.apply(cur.current, lo, hi);
        m.write(code, new);
        Ok(new)
    }

    pub fn set_vcp(&self, m: &Monitor, code: u8, value: Value) -> anyhow::Result<u32> {
        let new = match value {
            Value::Abs(v) => v,
            Value::Rel(_) => {
                let cur = m.current(code)?;
                value.apply(
                    cur.current,
                    0,
                    if cur.max == 0 { u32::MAX } else { cur.max },
                )
            }
        };
        m.write(code, new);
        Ok(new)
    }

    pub fn set_input(&self, m: &Monitor, name: &str) -> anyhow::Result<u32> {
        let v = vcp::lookup(vcp::INPUTS, name).ok_or_else(|| anyhow!("unknown input '{name}'"))?;
        m.write(vcp::INPUT_SOURCE, v);
        Ok(v)
    }

    pub fn set_color_preset(&self, m: &Monitor, name: &str) -> anyhow::Result<u32> {
        let v = vcp::lookup(vcp::COLOR_PRESETS, name)
            .ok_or_else(|| anyhow!("unknown color preset '{name}'"))?;
        m.write(vcp::COLOR_PRESET, v);
        Ok(v)
    }

    pub fn set_display_mode(&self, m: &Monitor, name: &str) -> anyhow::Result<u32> {
        let v = vcp::lookup(vcp::DISPLAY_MODES, name)
            .ok_or_else(|| anyhow!("unknown display mode '{name}'"))?;
        m.write(vcp::DISPLAY_MODE, v);
        Ok(v)
    }

    pub fn set_power(&self, m: &Monitor, on: bool) {
        m.write(
            vcp::POWER_MODE,
            if on {
                vcp::POWER_ON
            } else {
                vcp::POWER_OFF_DPM
            },
        );
    }

    pub fn toggle_power(&self, m: &Monitor) -> anyhow::Result<bool> {
        let on = m
            .read(vcp::POWER_MODE)
            .map(|v| PowerState::from_vcp(v.current) == PowerState::On);
        let turn_on = !on.unwrap_or(true);
        self.set_power(m, turn_on);
        Ok(turn_on)
    }

    pub fn factory_reset(&self, m: &Monitor) {
        m.write(vcp::RESTORE_FACTORY_DEFAULTS, 1);
        // Values change inside the monitor; re-read once it has settled.
        let tx = m.tx.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1500));
            let _ = tx.send(Job::Poll);
        });
    }

    pub fn set_system_volume(&self, value: Value) -> anyhow::Result<u32> {
        let (cur, muted) = platform::system_volume()?;
        let new = value.apply(cur, 0, 100);
        platform::set_system_volume(new)?;
        self.emit(Event::SystemVolume { volume: new, muted });
        Ok(new)
    }

    pub fn set_system_mute(&self, mute: bool) -> anyhow::Result<()> {
        platform::set_system_mute(mute)?;
        if let Ok((volume, muted)) = platform::system_volume() {
            self.emit(Event::SystemVolume { volume, muted });
        }
        Ok(())
    }

    /// Runs a command line. Blocking: call from a worker thread / `spawn_blocking`.
    pub fn execute(&self, line: &str) -> anyhow::Result<ExecOutcome> {
        let steps = match command::parse(line)? {
            Program::Steps(s) => s,
            Program::Toggle(alts) if alts.is_empty() => Vec::new(),
            Program::Toggle(alts) => {
                let mut toggles = self.0.toggles.lock();
                let idx = toggles.entry(line.trim().to_string()).or_insert(0);
                let chosen = alts[*idx % alts.len()].clone();
                *idx = (*idx + 1) % alts.len();
                chosen
            }
        };
        debug!("execute {line:?} -> {steps:?}");

        let mut out = ExecOutcome::default();
        let mut targets: Option<Vec<Arc<Monitor>>> = None;
        let mut prev_select = false;

        for step in steps {
            let is_select = matches!(step, Step::Select { .. });
            let resolve =
                |t: &Option<Vec<Arc<Monitor>>>| t.clone().unwrap_or_else(|| self.default_targets());
            match step {
                Step::Select { monitor } => {
                    let found = self.find(&monitor);
                    if found.is_empty() {
                        out.messages.push(format!("monitor '{monitor}' not found"));
                    }
                    match (&mut targets, prev_select) {
                        (Some(t), true) => t.extend(found),
                        _ => targets = Some(found),
                    }
                }
                Step::Adjust { feature, value } => {
                    let same_all = self.0.config.read().general.same_values_for_all
                        && matches!(feature, Feature::Brightness | Feature::Contrast);
                    let list = if same_all {
                        self.monitor_handles()
                    } else {
                        resolve(&targets)
                    };
                    for m in list {
                        if let Err(e) = self.adjust(&m, feature, value) {
                            out.messages.push(format!("{}: {e:#}", m.info.read().name));
                        }
                    }
                }
                Step::ColorPreset { name } => self.each(&resolve(&targets), &mut out, |m| {
                    self.set_color_preset(m, &name).map(drop)
                }),
                Step::DisplayMode { name } => self.each(&resolve(&targets), &mut out, |m| {
                    self.set_display_mode(m, &name).map(drop)
                }),
                Step::Input { name } => self.each(&resolve(&targets), &mut out, |m| {
                    self.set_input(m, &name).map(drop)
                }),
                Step::SetVcp { code, value } => self.each(&resolve(&targets), &mut out, |m| {
                    self.set_vcp(m, code, value).map(drop)
                }),
                Step::FactoryReset => self.each(&resolve(&targets), &mut out, |m| {
                    self.factory_reset(m);
                    Ok(())
                }),
                Step::Power => match &targets {
                    None => platform::all_monitors_off(),
                    Some(list) => self.each(list, &mut out, |m| self.toggle_power(m).map(drop)),
                },
                Step::Rotate => {
                    let mut seen = HashSet::new();
                    for m in resolve(&targets) {
                        let dev = m.info.read().gdi_device.clone();
                        if seen.insert(dev.clone()) {
                            if let Err(e) = platform::rotate_clockwise(&dev) {
                                out.messages.push(format!("rotate {dev}: {e:#}"));
                            }
                        }
                    }
                }
                Step::SystemVolume { value } => {
                    if let Err(e) = self.set_system_volume(value) {
                        out.messages.push(format!("volume: {e:#}"));
                    }
                }
                Step::Mute | Step::Unmute => {
                    if let Err(e) = self.set_system_mute(matches!(step, Step::Mute)) {
                        out.messages.push(format!("mute: {e:#}"));
                    }
                }
                Step::ReturnBrightness
                | Step::ReturnContrast
                | Step::ReturnVcp { .. }
                | Step::GetVcp { .. } => {
                    let Some(m) = resolve(&targets).into_iter().next() else {
                        out.messages.push("no monitor".into());
                        continue;
                    };
                    let code = match step {
                        Step::ReturnBrightness => m.code(Feature::Brightness),
                        Step::ReturnContrast => m.code(Feature::Contrast),
                        Step::ReturnVcp { code } | Step::GetVcp { code } => code,
                        _ => unreachable!(),
                    };
                    match m.read(code) {
                        Ok(v) => {
                            out.value = Some(v.current as i64);
                            if matches!(step, Step::GetVcp { .. }) {
                                out.messages.push(format!(
                                    "{}: VCP 0x{code:02X} ({}) current {} max {}",
                                    m.info.read().display_name(),
                                    vcp::vcp_name(code),
                                    v.current,
                                    v.max
                                ));
                            }
                        }
                        Err(e) => out.messages.push(format!("{e:#}")),
                    }
                }
                Step::Notify => {
                    let text = resolve(&targets)
                        .iter()
                        .map(|m| {
                            let s = m.snapshot();
                            format!(
                                "{}: brightness {} contrast {}",
                                s.display_name,
                                s.brightness
                                    .map(|v| v.current.to_string())
                                    .unwrap_or("-".into()),
                                s.contrast
                                    .map(|v| v.current.to_string())
                                    .unwrap_or("-".into())
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.emit(Event::Notification {
                        message: text.clone(),
                    });
                    out.messages.push(text);
                }
                Step::Pause { ms } => thread::sleep(Duration::from_millis(ms)),
                Step::Run { program } => {
                    if let Err(e) = run_program(&program) {
                        out.messages.push(format!("run '{program}': {e:#}"));
                    }
                }
                Step::Quit => {
                    out.quit = true;
                    self.emit(Event::Quit);
                }
                Step::Restart => {
                    out.restart = true;
                    self.emit(Event::Restart);
                }
                Step::Saturation { .. } | Step::Dimming { .. } | Step::SourceBrightness { .. } => {
                    out.messages
                        .push(format!("{step:?} is not implemented yet"));
                }
            }
            prev_select = is_select;
        }
        Ok(out)
    }

    fn each(
        &self,
        list: &[Arc<Monitor>],
        out: &mut ExecOutcome,
        f: impl Fn(&Monitor) -> anyhow::Result<()>,
    ) {
        for m in list {
            if let Err(e) = f(m) {
                out.messages
                    .push(format!("{}: {e:#}", m.info.read().display_name()));
            }
        }
    }
}

fn run_program(program: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .raw_arg(format!("/C start \"\" {program}"))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(program)
            .spawn()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;

    fn engine(n: usize) -> Engine {
        let e = Engine::new(
            Config::default(),
            None,
            Box::new(MockBackend::with_monitors(n)),
        );
        e.rescan().unwrap();
        // Wait for the initial poll.
        for m in e.monitor_handles() {
            m.read(vcp::BRIGHTNESS).unwrap();
        }
        e
    }

    fn brightness(e: &Engine, idx: usize) -> u32 {
        let m = e.find(&idx.to_string()).pop().unwrap();
        m.read(vcp::BRIGHTNESS).unwrap().current
    }

    #[test]
    fn absolute_and_relative() {
        let e = engine(2);
        e.execute("b 70").unwrap();
        assert_eq!(brightness(&e, 1), 70);
        assert_eq!(brightness(&e, 2), 50);
        e.execute("Mock2 b-15").unwrap();
        assert_eq!(brightness(&e, 2), 35);
        e.execute("2 b + 100").unwrap();
        assert_eq!(brightness(&e, 2), 100);
    }

    #[test]
    fn toggle_alternates() {
        let e = engine(1);
        e.execute("t b 0 t b 50").unwrap();
        assert_eq!(brightness(&e, 1), 0);
        e.execute("t b 0 t b 50").unwrap();
        assert_eq!(brightness(&e, 1), 50);
        e.execute("t b 0 t b 50").unwrap();
        assert_eq!(brightness(&e, 1), 0);
    }

    #[test]
    fn prefix_selector_hits_all_matches() {
        let e = engine(3);
        e.execute("Mo b 10").unwrap();
        assert_eq!(
            (brightness(&e, 1), brightness(&e, 2), brightness(&e, 3)),
            (10, 10, 10)
        );
    }

    #[test]
    fn return_values_and_inputs() {
        let e = engine(1);
        e.execute("b 42").unwrap();
        assert_eq!(e.execute("d").unwrap().value, Some(42));
        e.execute("s hdmi2").unwrap();
        assert_eq!(e.execute("get_VCP 0x60").unwrap().value, Some(0x12));
    }

    #[test]
    fn limits_apply() {
        let e = engine(1);
        let id = e.monitor_handles()[0].id();
        e.update_config(|c| c.monitor_mut(&id).max_brightness = Some(80))
            .unwrap();
        e.execute("b 100").unwrap();
        assert_eq!(brightness(&e, 1), 80);
    }

    #[test]
    fn unknown_monitor_reports() {
        let e = engine(1);
        let out = e.execute("Nope b 10").unwrap();
        assert!(out.messages[0].contains("not found"));
        assert_eq!(brightness(&e, 1), 50);
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumenctl-engine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn restore_writes_saved_brightness_and_ignores_a_later_read() {
        let dir = temp_dir();
        let backend = MockBackend::with_monitors(1);
        let values = backend.monitors[0].1.clone();
        let mut cfg = Config::default();
        cfg.general.restore_values = true;
        let e = Engine::new(cfg, Some(dir.join("config.toml")), Box::new(backend));
        e.rescan().unwrap();
        e.execute("b 22").unwrap();
        assert_eq!(brightness(&e, 1), 22);
        let saved = std::fs::read_to_string(dir.join("state.toml")).unwrap();
        assert!(saved.contains("brightness = 22"), "{saved}");

        values.lock().get_mut(&vcp::BRIGHTNESS).unwrap().current = 80;
        let m = e.monitor_handles().remove(0);
        m.refresh();
        assert_eq!(m.read(vcp::BRIGHTNESS).unwrap().current, 80);
        let saved = std::fs::read_to_string(dir.join("state.toml")).unwrap();
        assert!(saved.contains("brightness = 22"), "{saved}");

        e.restore_with(1, Duration::ZERO);
        assert_eq!(values.lock()[&vcp::BRIGHTNESS].current, 22);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_reloads_state_file_for_a_new_process() {
        let dir = temp_dir();
        std::fs::write(
            dir.join("state.toml"),
            "[[monitor]]\nid = \"MOCK0000#1&0&UID0\"\nbrightness = 22\ncontrast = 40\n",
        )
        .unwrap();
        let backend = MockBackend::with_monitors(1);
        let values = backend.monitors[0].1.clone();
        let mut cfg = Config::default();
        cfg.general.restore_values = true;
        let e = Engine::new(cfg, Some(dir.join("config.toml")), Box::new(backend));
        e.rescan().unwrap();
        e.restore_with(1, Duration::ZERO);
        assert_eq!(values.lock()[&vcp::BRIGHTNESS].current, 22);
        assert_eq!(values.lock()[&vcp::CONTRAST].current, 40);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabling_restore_snapshots_current_values() {
        let dir = temp_dir();
        let e = Engine::new(
            Config::default(),
            Some(dir.join("config.toml")),
            Box::new(MockBackend::with_monitors(1)),
        );
        e.rescan().unwrap();
        let _ = brightness(&e, 1);
        e.update_config(|c| c.general.restore_values = true)
            .unwrap();
        let saved = std::fs::read_to_string(dir.join("state.toml")).unwrap();
        assert!(saved.contains("brightness = 50"), "{saved}");
        assert!(saved.contains("contrast = 50"), "{saved}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_monitor_does_not_erase_saved_values() {
        use crate::backend::{Backend, Device, Discovered};

        struct Silent;
        impl Device for Silent {
            fn get_vcp(&mut self, code: u8) -> anyhow::Result<VcpValue> {
                anyhow::bail!("offline 0x{code:02X}")
            }
            fn set_vcp(&mut self, code: u8, value: u32) -> anyhow::Result<()> {
                anyhow::bail!("offline 0x{code:02X}={value}")
            }
            fn capabilities(&mut self) -> anyhow::Result<String> {
                anyhow::bail!("offline")
            }
            fn save_settings(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
        }
        struct One;
        impl Backend for One {
            fn enumerate(&self) -> anyhow::Result<Vec<Discovered>> {
                Ok(vec![Discovered {
                    info: crate::model::MonitorInfo {
                        id: "MOCK0000#1&0&UID0".into(),
                        index: 1,
                        name: "Silent".into(),
                        ..Default::default()
                    },
                    device: Box::new(Silent),
                }])
            }
        }

        let dir = temp_dir();
        let body = "[[monitor]]\nid = \"MOCK0000#1&0&UID0\"\nbrightness = 30\ncontrast = 40\n";
        std::fs::write(dir.join("state.toml"), body).unwrap();
        let mut cfg = Config::default();
        cfg.general.restore_values = true;
        let e = Engine::new(cfg, Some(dir.join("config.toml")), Box::new(One));
        e.rescan().unwrap();
        e.restore_with(1, Duration::ZERO);
        assert_eq!(
            std::fs::read_to_string(dir.join("state.toml")).unwrap(),
            body
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
