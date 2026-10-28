# lumenctl

Open-source replacement for [ClickMonitorDDC](https://clickmonitorddc.bplaced.net): control the real
backlight brightness, contrast, colors, input and power of external monitors over DDC/CI — from the
tray, a modern UI, the command line, HTTP/WebSocket or Home Assistant.

- **Tray icons with numbers** for every monitor; scroll over an icon to change brightness, middle click
  toggles two values, right click opens a menu (power, inputs, your own commands).
- **Fast**: Rust core, one worker thread per monitor, optimistic UI updates, write coalescing
  (fast scrolling sends only the final value) and de-duplicated writes to spare the monitor EEPROM.
- **ClickMonitorDDC command language** (`MateView b-10 N2724A b+10`, `t b 0 t b 50`, `setVCP 0x10 + 10`, …).
- **REST + WebSocket API**, optional bearer token.
- **MQTT**: Home Assistant discovery, or the HACS integration in `custom_components/lumenctl`.
  Every monitor is a device with a light (brightness + power), numbers (contrast, volume, RGB)
  and selects (input, color preset, mode).

## Build & run

Requirements: Windows 10/11, [Rust](https://rustup.rs) (MSVC), Node.js 20+.

```powershell
npm install
npm run dev      # development (hot reload UI)
npm run build    # release build + NSIS/MSI installers in target/release/bundle
```

Configuration lives in `%APPDATA%\lumenctl\config.toml` (editable from the Settings screen).
Last brightness and contrast are kept beside it in `state.toml`.

## Download

Windows builds are on the [releases page](https://github.com/choopachuck/lumenctl/releases).
`lumenctl-<version>-x64-setup.exe` is the installer. `lumenctl-<version>-x64.exe` is the program itself.

A release is built when a `v*` tag is pushed:

```powershell
git tag v1.0.0
git push origin v1.0.0
```

## Command language

Compatible with ClickMonitorDDC. Prefix commands with a monitor name, alias, serial, number or id;
without a prefix the default monitor (or the monitor under the cursor, if enabled) is used.

| Command | Meaning |
|---|---|
| `b 50`, `b + 10`, `b-5` | brightness absolute / relative |
| `c`, `r`, `g`, `l` (`bl`, `i`) | contrast, red, green, blue gain |
| `o` | monitor speaker volume |
| `v`, `m`, `u` | system volume, mute, unmute |
| `p 6500K`, `p User1` | color preset (VCP 0x14) |
| `j Movie` | display mode / preset (VCP 0xDC) |
| `s HDMI1`, `s DisplayPort2`, `s 0x11` | input source |
| `x` | all monitors to power-save; with a monitor prefix: toggle that monitor (VCP 0xD6) |
| `w` | rotate display 90° clockwise |
| `f` | factory reset |
| `t A t B` | alternate between programs A and B on each run |
| `;` `,` | pause 100 ms / 1000 ms |
| `d`, `e`, `get_VCP 0x12` | return brightness / contrast / VCP value (exit code on the CLI) |
| `getVCP 0x12`, `setVCP 0x10 + 10` | raw VCP access |
| `! notepad.exe` | start a program |
| `#` | comment (rest is ignored) |
| `q`, `z` | quit / restart lumenctl |

```powershell
lumenctl.exe MateView b 30 N2724A b+10   # forwarded to the running instance
lumenctl.exe d; echo $LASTEXITCODE        # current brightness as exit code
```

## HTTP / WebSocket API

Default `http://127.0.0.1:7685`. Set `api.bind = "0.0.0.0:7685"` and `api.token` to reach it from
other machines. With a token send `Authorization: Bearer <token>` (or `?token=` for WebSockets).
`{ref}` is the monitor `key`, `id`, number, name or alias.

| Method | Path | Body |
|---|---|---|
| GET | `/api/monitors` | |
| GET / PATCH | `/api/monitors/{ref}` | `{"brightness": 40, "contrast": "+5", "input": "HDMI1", "color_preset": "6500K", "power": true}` |
| POST | `/api/monitors/{ref}/refresh` | re-read values from the monitor |
| GET | `/api/monitors/{ref}/capabilities?refresh=true` | |
| GET / PUT | `/api/monitors/{ref}/vcp/{code}` | `{"value": 50}` or `{"value": "-10"}` |
| POST | `/api/command` | `{"command": "MateView b 30"}` |
| POST | `/api/rescan` | |
| GET / PUT | `/api/system/volume` | `{"volume": 30, "muted": false}` |
| GET | `/api/ws` | WebSocket |

The WebSocket first sends `{"type":"monitors_changed","monitors":[…]}`, then pushes
`monitor_updated`, `system_volume`, `notification` and `config_changed` events. Clients can send:

```json
{"type": "command", "command": "b 40", "id": 1}
{"type": "set", "monitor": "1", "brightness": 40, "id": 2}
{"type": "rescan"}
```

## Home Assistant

lumenctl talks to Home Assistant over MQTT. Use one of the two entity sources, not both.

### HACS integration

In HACS, add this repository as an Integration and install lumenctl. Home Assistant needs the MQTT
integration configured first. Then add the lumenctl integration: the default base topic is `lumenctl`.
Leave node id empty to pick up every computer, or set the node id shown in lumenctl Settings.

In lumenctl, enable MQTT. The broker can be `192.168.2.95` or `mqtt://192.168.2.95`; a port in that address overrides the port field. Turn **Publish Home Assistant MQTT discovery** off. Discovery left on
creates a second copy of every entity. The topic contract is in
[docs/hacs-integration.md](docs/hacs-integration.md).

Each monitor becomes a device: a light (power and brightness), sliders for contrast, volume and RGB
(RGB is hidden until enabled), selects for input, color preset and display mode, and a DDC/CI
diagnostic. The PC is a device with a command field and a Rescan monitors button.

### MQTT discovery

Leave **Publish Home Assistant MQTT discovery** on (the default) and set the discovery prefix to
`homeassistant`. Entities appear without the custom integration: `light.<monitor>` (brightness,
on/off = monitor power), `number.*_contrast`, `number.*_volume`, `select.*_input`,
`select.*_color_preset`, plus a text entity that runs any command line and a Rescan monitors button.

### REST

Without MQTT you can use the REST API directly:

```yaml
rest_command:
  monitors_night:
    url: http://my-pc:7685/api/command
    method: POST
    headers: { Authorization: "Bearer YOUR_TOKEN" }
    content_type: application/json
    payload: '{"command": "b 10 c 30"}'
```

## Status vs. ClickMonitorDDC 7.2

| Feature | Status |
|---|---|
| DDC/CI brightness, contrast, RGB gain, speaker volume, input, color preset, display mode, power, factory reset | ✅ |
| Tray icons per monitor with scroll, middle-click toggle, context menu, custom menu entries | ✅ |
| Command language, toggles, pauses, raw VCP, return values / CLI forwarding | ✅ |
| Per-monitor aliases, brightness/contrast limits, VCP 0x6B, ignore device, icon fonts and colors | ✅ |
| Monitor info (EDID name, serial, year, connection), capability string, VCP explorer | ✅ |
| Verify writes, save to NVRAM, same values for all monitors, monitor-under-cursor default | ✅ |
| System volume, rotate display, all-monitors power-save, start with Windows | ✅ |
| HTTP/WS API, MQTT discovery, HACS integration | ✅ new |
| Global hotkeys and wheel + modifier combinations | planned |
| Timed auto-run commands, per-program / fullscreen auto-run with restore | planned |
| Dim after inactivity, brightness transfer from a source monitor | planned |
| Restore last brightness and contrast on start / after wake-up | ✅ |
| NVIDIA Digital Vibrance / AMD saturation (`k`) | planned |
| Laptop panel brightness (WMI) | planned |
| On-screen notifications for hotkeys, import of `ClickMonitorDDC.xml` | planned |

## Layout

```
crates/lumen-core    engine, DDC/CI backend (dxva2), command language, config
crates/lumen-server  axum REST/WS API, MQTT bridge
src-tauri            desktop app: Win32 tray, Tauri commands, CLI forwarding
ui                   Svelte 5 frontend
custom_components/lumenctl   Home Assistant integration installed through HACS
```

## License

MIT
