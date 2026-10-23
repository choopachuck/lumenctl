<script lang="ts">
  import { onMount } from "svelte";
  import { api, type AppInfo, type Config, type MonitorConfig } from "../lib/api";
  import { app, toast } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";

  let draft = $state<Config | null>(null);
  let info = $state<AppInfo | null>(null);
  let saving = $state(false);

  onMount(async () => {
    const cfg = structuredClone(await api.config());
    for (const m of app.monitors) {
      if (!cfg.monitor.some((c) => c.id === m.id)) {
        cfg.monitor.push({
          id: m.id, alias: null, ignore: false, use_6b: false, show_brightness_icon: null, show_contrast_icon: null,
          min_brightness: null, max_brightness: null, min_contrast: null, max_contrast: null, font: null,
        });
      }
    }
    draft = cfg;
    info = await api.info();
  });

  function monitorCfg(id: string): MonitorConfig {
    return draft!.monitor.find((m) => m.id === id)!;
  }

  function numOrNull(v: string) {
    return v.trim() === "" ? null : Math.max(0, Math.min(100, +v));
  }

  function generateToken() {
    const bytes = crypto.getRandomValues(new Uint8Array(18));
    draft!.api.token = btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, "");
  }

  async function save() {
    if (!draft) return;
    saving = true;
    try {
      const clean = $state.snapshot(draft) as Config;
      for (const k of ["token"] as const) if (!clean.api[k]) clean.api[k] = null;
      await api.saveConfig(clean);
      toast("Settings saved");
    } catch (e) {
      toast(String(e));
    } finally {
      saving = false;
    }
  }
</script>

{#if draft}
  <div class="scroll">
    <h2>General</h2>
    <div class="card form">
      <label><input type="checkbox" bind:checked={draft.general.start_with_windows} /> Start with Windows</label>
      <label><input type="checkbox" bind:checked={draft.general.target_monitor_under_cursor} />
        Commands without a monitor name target the monitor under the cursor</label>
      <label><input type="checkbox" bind:checked={draft.general.same_values_for_all} />
        Same brightness/contrast for all monitors</label>
      <label><input type="checkbox" bind:checked={draft.general.verify_writes} /> Verify values after writing (+50 ms)</label>
      <label><input type="checkbox" bind:checked={draft.general.save_to_nvram} />
        Force monitors to store values (for monitors that forget them, +200 ms)</label>
      <label><input type="checkbox" bind:checked={draft.tray.hide_window_on_blur} /> Hide this window when it loses focus</label>
    </div>

    <h2>Tray icons</h2>
    <div class="card form">
      <label><input type="checkbox" bind:checked={draft.tray.enabled} /> Show tray icons</label>
      <label><input type="checkbox" bind:checked={draft.tray.show_system_volume_icon} /> System volume icon</label>
      <div class="grid">
        <span>Wheel step: brightness</span><input type="number" min="1" max="50" bind:value={draft.tray.wheel_step_brightness} />
        <span>Wheel step: contrast</span><input type="number" min="1" max="50" bind:value={draft.tray.wheel_step_contrast} />
        <span>Wheel step: volume</span><input type="number" min="1" max="50" bind:value={draft.tray.wheel_step_volume} />
        <span>Middle click brightness</span>
        <div class="row"><input type="number" min="0" max="100" bind:value={draft.tray.middle_click_brightness[0]} />
          ↔ <input type="number" min="0" max="100" bind:value={draft.tray.middle_click_brightness[1]} /></div>
        <span>Middle click contrast</span>
        <div class="row"><input type="number" min="0" max="100" bind:value={draft.tray.middle_click_contrast[0]} />
          ↔ <input type="number" min="0" max="100" bind:value={draft.tray.middle_click_contrast[1]} /></div>
        <span>Font</span><input type="text" bind:value={draft.tray.font} />
        <span>Colors</span>
        <div class="row">
          <input type="color" title="Brightness" bind:value={draft.tray.brightness_color} />
          <input type="color" title="Contrast" bind:value={draft.tray.contrast_color} />
          <input type="color" title="Volume" bind:value={draft.tray.volume_color} />
        </div>
      </div>
    </div>

    <h2>Monitors</h2>
    {#each app.monitors as m (m.id)}
      {@const mc = monitorCfg(m.id)}
      <div class="card form">
        <div class="row"><strong class="grow">{m.index}. {m.display_name}</strong><span class="muted">{m.output}</span></div>
        <div class="grid">
          <span>Name</span><input type="text" placeholder={m.name} value={mc.alias ?? ""}
            oninput={(e) => (mc.alias = e.currentTarget.value || null)} />
          <span>Tray icons</span>
          <div class="row">
            <label><input type="checkbox" checked={mc.show_brightness_icon ?? true}
              onchange={(e) => (mc.show_brightness_icon = e.currentTarget.checked)} /> Brightness</label>
            <label><input type="checkbox" checked={mc.show_contrast_icon ?? false}
              onchange={(e) => (mc.show_contrast_icon = e.currentTarget.checked)} /> Contrast</label>
          </div>
          <span>Brightness limits</span>
          <div class="row">
            <input type="number" placeholder="0" value={mc.min_brightness ?? ""} oninput={(e) => (mc.min_brightness = numOrNull(e.currentTarget.value))} />
            – <input type="number" placeholder="100" value={mc.max_brightness ?? ""} oninput={(e) => (mc.max_brightness = numOrNull(e.currentTarget.value))} />
          </div>
          <span>Contrast limits</span>
          <div class="row">
            <input type="number" placeholder="0" value={mc.min_contrast ?? ""} oninput={(e) => (mc.min_contrast = numOrNull(e.currentTarget.value))} />
            – <input type="number" placeholder="100" value={mc.max_contrast ?? ""} oninput={(e) => (mc.max_contrast = numOrNull(e.currentTarget.value))} />
          </div>
          <span>Icon font</span><input type="text" placeholder={draft.tray.font} value={mc.font ?? ""}
            oninput={(e) => (mc.font = e.currentTarget.value || null)} />
        </div>
        <label><input type="checkbox" bind:checked={mc.use_6b} /> Use VCP 0x6B for brightness</label>
        <label><input type="checkbox" bind:checked={mc.ignore} /> Ignore this device (no DDC/CI)</label>
      </div>
    {/each}

    <h2>Tray menu commands</h2>
    <div class="card form">
      {#each draft.menu as entry, i}
        <div class="row">
          <input type="text" class="label" placeholder="Label" bind:value={entry.label} />
          <input type="text" class="grow mono" placeholder="b 30 c 40" bind:value={entry.command} />
          <button class="icon" title="Remove" onclick={() => draft!.menu.splice(i, 1)}><Icon name="trash" size={14} /></button>
        </div>
      {/each}
      <div><button onclick={() => draft!.menu.push({ label: "", command: "" })}><Icon name="plus" size={14} /> Add</button></div>
    </div>

    <h2>HTTP / WebSocket API</h2>
    <div class="card form">
      <label><input type="checkbox" bind:checked={draft.api.enabled} /> Enable API</label>
      <div class="grid">
        <span>Listen on</span><input type="text" class="mono" bind:value={draft.api.bind} />
        <span>Token</span>
        <div class="row"><input type="text" class="grow mono" placeholder="none" bind:value={draft.api.token} />
          <button onclick={generateToken}>Generate</button></div>
      </div>
      <p class="muted">Use <code>0.0.0.0:7685</code> to allow access from other machines (set a token then).
        Endpoints: <code>/api/monitors</code>, <code>/api/command</code>, <code>/api/ws</code>.</p>
    </div>

    <h2>MQTT · Home Assistant</h2>
    <div class="card form">
      <label><input type="checkbox" bind:checked={draft.mqtt.enabled} /> Enable MQTT with Home Assistant discovery</label>
      <div class="grid">
        <span>Broker</span>
        <div class="row"><input type="text" class="grow" bind:value={draft.mqtt.host} />
          <input type="number" class="port" bind:value={draft.mqtt.port} /></div>
        <span>Username</span><input type="text" value={draft.mqtt.username ?? ""} oninput={(e) => (draft!.mqtt.username = e.currentTarget.value || null)} />
        <span>Password</span><input type="password" value={draft.mqtt.password ?? ""} oninput={(e) => (draft!.mqtt.password = e.currentTarget.value || null)} />
        <span>Discovery prefix</span><input type="text" bind:value={draft.mqtt.discovery_prefix} />
        <span>Base topic</span><input type="text" bind:value={draft.mqtt.base_topic} />
        <span>Node id</span><input type="text" placeholder={info?.mqtt_node_id} value={draft.mqtt.node_id ?? ""}
          oninput={(e) => (draft!.mqtt.node_id = e.currentTarget.value || null)} />
      </div>
    </div>

    {#if info}<p class="muted foot">lumenctl {info.version} · {info.config_path}</p>{/if}
  </div>
  <footer class="row">
    <span class="grow"></span>
    <button onclick={() => (app.view = "monitors")}>Cancel</button>
    <button class="primary" disabled={saving} onclick={save}>Save</button>
  </footer>
{/if}

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .form label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grid {
    display: grid;
    grid-template-columns: 42% 1fr;
    gap: 6px 10px;
    align-items: center;
  }
  .grid > span {
    color: var(--text-2);
  }
  .grid input[type="number"] {
    width: 70px;
  }
  .grid .port {
    width: 80px;
  }
  input[type="color"] {
    width: 34px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: none;
  }
  .label {
    width: 110px;
  }
  .mono,
  code {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
  }
  p {
    margin: 0;
  }
  .foot {
    font-size: 11px;
    margin-top: 10px;
    user-select: text;
  }
  footer {
    padding: 10px 12px;
    border-top: 1px solid var(--border);
  }
</style>
