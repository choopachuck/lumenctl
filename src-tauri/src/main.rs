#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod commands;
#[cfg(windows)]
mod tray;

use std::sync::Mutex;

use lumen_core::backend::default_backend;
use lumen_core::{Config, Engine, Event};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tokio::sync::broadcast::error::RecvError;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hidden = args.iter().any(|a| cli::HIDDEN_FLAGS.contains(&a.as_str()));
    let command = cli::command_from_args(&args);
    let show_on_start = !hidden && command.is_none();

    let config_path = Config::default_path();
    let config = Config::load(&config_path).unwrap_or_else(|e| {
        eprintln!(
            "cannot read {}: {e:#}; using defaults",
            config_path.display()
        );
        Config::default()
    });

    if let Some(line) = &command {
        cli::attach_console();
        if let Ok(out) = cli::forward(&config, line) {
            for m in out["messages"].as_array().into_iter().flatten() {
                println!("{}", m.as_str().unwrap_or_default());
            }
            std::process::exit(out["value"].as_i64().unwrap_or(0) as i32);
        }
        // No running instance: start normally and run the command once monitors are known.
    }

    init_logging(&config_path);
    let engine = Engine::new(config, Some(config_path), default_backend());
    {
        let engine = engine.clone();
        let command = command.clone();
        std::thread::spawn(move || {
            if let Err(e) = engine.rescan() {
                tracing::error!("rescan: {e:#}");
            }
            engine.restore_saved();
            if let Some(line) = command {
                run_command(&engine, &line);
            }
        });
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, argv, _cwd| match cli::command_from_args(&argv[1.min(argv.len())..]) {
                Some(line) => {
                    let engine = app.state::<Engine>().inner().clone();
                    std::thread::spawn(move || run_command(&engine, &line));
                }
                None => show_window(app),
            },
        ))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(engine.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_monitors,
            commands::patch_monitor,
            commands::execute_command,
            commands::rescan,
            commands::refresh_monitor,
            commands::get_capabilities,
            commands::read_vcp,
            commands::write_vcp,
            commands::monitor_action,
            commands::select_monitor,
            commands::get_config,
            commands::save_config,
            commands::test_mqtt,
            commands::get_system_volume,
            commands::set_system_volume,
            commands::app_info,
            commands::list_fonts,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let mut rx = engine.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(Event::Quit) => handle.exit(0),
                        Ok(Event::Restart) => handle.restart(),
                        Ok(ev) => {
                            let _ = handle.emit("lumen-event", &ev);
                        }
                        Err(RecvError::Lagged(_)) => {
                            let ev = Event::MonitorsChanged {
                                monitors: handle.state::<Engine>().monitors(),
                            };
                            let _ = handle.emit("lumen-event", &ev);
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            tauri::async_runtime::spawn(lumen_server::supervise(engine.clone()));

            #[cfg(windows)]
            {
                let open = app.handle().clone();
                let quit = app.handle().clone();
                tray::spawn(
                    engine.clone(),
                    tray::Hooks {
                        open: Box::new(move || toggle_window(&open)),
                        quit: Box::new(move || quit.exit(0)),
                    },
                );
            }
            if show_on_start {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(false) => {
                let engine = window.state::<Engine>();
                if engine.config().tray.hide_window_on_blur && !cfg!(debug_assertions) {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building lumenctl");

    app.run(|_app, event| match event {
        RunEvent::ExitRequested {
            api, code: None, ..
        } => api.prevent_exit(),
        RunEvent::Exit => {
            #[cfg(windows)]
            tray::shutdown();
        }
        _ => {}
    });
}

fn run_command(engine: &Engine, line: &str) {
    match engine.execute(line) {
        Ok(out) => {
            for m in out.messages {
                tracing::info!("{m}");
            }
        }
        Err(e) => tracing::warn!("command '{line}': {e:#}"),
    }
}

fn toggle_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
            return;
        }
    }
    show_window(app);
}

/// Shows the window as a flyout above the notification area of the monitor under the cursor.
fn show_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    if let (Ok(cursor), Ok(size)) = (app.cursor_position(), w.outer_size()) {
        if let Ok(Some(mon)) = app.monitor_from_point(cursor.x, cursor.y) {
            let area = mon.work_area();
            let margin = (12.0 * mon.scale_factor()) as i32;
            let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
            let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    }
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
}

fn init_logging(config_path: &std::path::Path) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let log_path = config_path.with_file_name("lumenctl.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(std::path::Path::new(".")));
    match std::fs::File::create(&log_path) {
        Ok(file) if !cfg!(debug_assertions) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        _ => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}
