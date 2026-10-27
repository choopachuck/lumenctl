import { invoke } from "@tauri-apps/api/core";

export interface VcpValue {
  current: number;
  max: number;
}

export interface NamedValue {
  value: number;
  name: string;
}

export interface Monitor {
  id: string;
  key: string;
  index: number;
  name: string;
  alias: string | null;
  display_name: string;
  manufacturer_id: string | null;
  product_code: string | null;
  serial: string | null;
  year: number | null;
  week: number | null;
  gdi_device: string;
  description: string;
  primary: boolean;
  connection: "external" | "internal";
  output: string | null;
  ddc_ok: boolean;
  brightness: VcpValue | null;
  contrast: VcpValue | null;
  red: VcpValue | null;
  green: VcpValue | null;
  blue: VcpValue | null;
  volume: VcpValue | null;
  input: NamedValue | null;
  color_preset: NamedValue | null;
  display_mode: NamedValue | null;
  power: "on" | "standby" | "off" | null;
  supported_inputs: NamedValue[];
  supported_color_presets: NamedValue[];
  supported_display_modes: NamedValue[];
  last_error: string | null;
}

export type ValueSpec = number | string;

export interface MonitorPatch {
  brightness?: ValueSpec;
  contrast?: ValueSpec;
  red?: ValueSpec;
  green?: ValueSpec;
  blue?: ValueSpec;
  volume?: ValueSpec;
  input?: number | string;
  color_preset?: number | string;
  display_mode?: number | string;
  power?: boolean;
}

export interface ExecOutcome {
  messages: string[];
  value: number | null;
  quit: boolean;
  restart: boolean;
}

export interface Capabilities {
  raw: string;
  model: string | null;
  kind: string | null;
  mccs_version: string | null;
  vcp: Record<string, number[]>;
}

export interface MonitorConfig {
  id: string;
  alias: string | null;
  ignore: boolean;
  use_6b: boolean;
  show_brightness_icon: boolean | null;
  show_contrast_icon: boolean | null;
  min_brightness: number | null;
  max_brightness: number | null;
  min_contrast: number | null;
  max_contrast: number | null;
  font: string | null;
  brightness_color: string | null;
  contrast_color: string | null;
}

export interface Config {
  general: {
    target_monitor_under_cursor: boolean;
    default_monitor: string | null;
    restore_values: boolean;
    verify_writes: boolean;
    save_to_nvram: boolean;
    same_values_for_all: boolean;
    start_with_windows: boolean;
  };
  api: { enabled: boolean; bind: string; token: string | null };
  mqtt: {
    enabled: boolean;
    host: string;
    port: number;
    username: string | null;
    password: string | null;
    discovery_prefix: string;
    base_topic: string;
    node_id: string | null;
    discovery: boolean;
  };
  tray: {
    enabled: boolean;
    wheel_step_brightness: number;
    wheel_step_contrast: number;
    wheel_step_volume: number;
    middle_click_brightness: [number, number];
    middle_click_contrast: [number, number];
    font: string;
    brightness_color: string;
    contrast_color: string;
    volume_color: string;
    show_system_volume_icon: boolean;
    hide_window_on_blur: boolean;
  };
  monitor: MonitorConfig[];
  menu: { label: string; command: string }[];
}

export interface AppInfo {
  version: string;
  config_path: string;
  mqtt_node_id: string;
}

export type LumenEvent =
  | { type: "monitors_changed"; monitors: Monitor[] }
  | { type: "monitor_updated"; monitor: Monitor }
  | { type: "system_volume"; volume: number; muted: boolean }
  | { type: "notification"; message: string }
  | { type: "config_changed" };

export const api = {
  monitors: () => invoke<Monitor[]>("get_monitors"),
  patch: (reference: string, patch: MonitorPatch) => invoke<Monitor>("patch_monitor", { reference, patch }),
  execute: (command: string) => invoke<ExecOutcome>("execute_command", { command }),
  rescan: () => invoke<Monitor[]>("rescan"),
  refresh: (reference: string) => invoke<void>("refresh_monitor", { reference }),
  capabilities: (reference: string, refresh = false) =>
    invoke<Capabilities>("get_capabilities", { reference, refresh }),
  readVcp: (reference: string, code: number) => invoke<VcpValue>("read_vcp", { reference, code }),
  writeVcp: (reference: string, code: number, value: ValueSpec) =>
    invoke<number>("write_vcp", { reference, code, value }),
  action: (reference: string, action: "factory_reset" | "rotate" | "power_toggle") =>
    invoke<void>("monitor_action", { reference, action }),
  select: (id: string | null) => invoke<void>("select_monitor", { id }),
  config: () => invoke<Config>("get_config"),
  saveConfig: (config: Config) => invoke<void>("save_config", { config }),
  systemVolume: () => invoke<{ volume: number; muted: boolean }>("get_system_volume"),
  setSystemVolume: (volume?: ValueSpec, muted?: boolean) => invoke<void>("set_system_volume", { volume, muted }),
  info: () => invoke<AppInfo>("app_info"),
  fonts: () => invoke<string[]>("list_fonts"),
};

export const VCP_NAMES: Record<number, string> = {
  0x02: "New Control Value",
  0x04: "Restore Factory Defaults",
  0x05: "Restore Luminance/Contrast",
  0x08: "Restore Factory Color",
  0x10: "Brightness",
  0x12: "Contrast",
  0x14: "Color Preset",
  0x16: "Red Gain",
  0x18: "Green Gain",
  0x1a: "Blue Gain",
  0x52: "Active Control",
  0x60: "Input Source",
  0x62: "Speaker Volume",
  0x6b: "Backlight White",
  0x72: "Gamma",
  0x87: "Sharpness",
  0x8a: "Color Saturation",
  0x8d: "Audio Mute",
  0xaa: "Screen Orientation",
  0xac: "Horizontal Frequency",
  0xae: "Vertical Frequency",
  0xb2: "Sub-Pixel Layout",
  0xb6: "Display Technology",
  0xc0: "Usage Time",
  0xc6: "Application Enable Key",
  0xc8: "Controller Type",
  0xc9: "Firmware Level",
  0xca: "OSD",
  0xcc: "OSD Language",
  0xd6: "Power Mode",
  0xdc: "Display Mode",
  0xdf: "VCP Version",
};

export const hex = (n: number) => "0x" + n.toString(16).toUpperCase().padStart(2, "0");
