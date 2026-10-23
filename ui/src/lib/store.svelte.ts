import { listen } from "@tauri-apps/api/event";
import { api, type Config, type LumenEvent, type Monitor, type MonitorPatch } from "./api";

export type View = "monitors" | "settings" | "console" | "vcp";

export const app = $state({
  monitors: [] as Monitor[],
  config: null as Config | null,
  view: "monitors" as View,
  vcpMonitor: null as string | null,
  linkAll: false,
  toast: "",
});

let toastTimer: ReturnType<typeof setTimeout> | undefined;

export function toast(message: string) {
  app.toast = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (app.toast = ""), 3500);
}

function onEvent(ev: LumenEvent) {
  switch (ev.type) {
    case "monitors_changed":
      app.monitors = ev.monitors;
      break;
    case "monitor_updated": {
      const i = app.monitors.findIndex((m) => m.id === ev.monitor.id);
      if (i >= 0) app.monitors[i] = ev.monitor;
      break;
    }
    case "notification":
      toast(ev.message);
      break;
    case "config_changed":
      api.config().then((c) => (app.config = c));
      break;
  }
}

export async function init() {
  await listen<LumenEvent>("lumen-event", (e) => onEvent(e.payload));
  app.monitors = await api.monitors();
  app.config = await api.config();
}

/** Applies a change to one monitor, or to all monitors when "link" is on. */
export function patch(monitor: Monitor, p: MonitorPatch) {
  const targets = app.linkAll && ("brightness" in p || "contrast" in p) ? app.monitors : [monitor];
  for (const m of targets) {
    api.patch(m.key, p).catch((e) => toast(String(e)));
  }
}

export async function saveConfig(mutate: (c: Config) => void) {
  if (!app.config) return;
  const next: Config = structuredClone($state.snapshot(app.config));
  mutate(next);
  try {
    await api.saveConfig(next);
    app.config = next;
  } catch (e) {
    toast(String(e));
  }
}
