<script lang="ts">
  import { onMount } from "svelte";
  import { api, type AppInfo, type Config, type MonitorConfig } from "../lib/api";
  import { app, toast } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";

  let draft = $state<Config | null>(null);
  let info = $state<AppInfo | null>(null);
  let fonts = $state<string[]>([]);
  let saving = $state(false);
  let testing = $state(false);
  let mqttTest = $state<{ ok: boolean; text: string } | null>(null);

  onMount(async () => {
    fonts = await api.fonts().catch(() => []);
    const cfg = structuredClone(await api.config());
    cfg.mqtt.discovery ??= true;
    for (const m of app.monitors) {
      if (!cfg.monitor.some((c) => c.id === m.id)) {
        cfg.monitor.push({
          id: m.id, alias: null, ignore: false, use_6b: false, show_brightness_icon: null, show_contrast_icon: null,
          min_brightness: null, max_brightness: null, min_contrast: null, max_contrast: null,
          font: null, brightness_color: null, contrast_color: null,
        });
      }
    }
    draft = cfg;
    info = await api.info();
  });

  function monitorCfg(id: string): MonitorConfig {
    return draft!.monitor.find((m) => m.id === id)!;
  }

  function fontOptions(current: string | null | undefined) {
    const list = [...fonts];
    if (current && !list.includes(current)) list.unshift(current);
    return list;
  }

  function numOrNull(v: string) {
    return v.trim() === "" ? null : Math.max(0, Math.min(100, +v));
  }

  function generateToken() {
    const bytes = crypto.getRandomValues(new Uint8Array(18));
    draft!.api.token = btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, "");
  }

  function errorText(e: unknown) {
    if (typeof e === "string") return e;
    if (e instanceof Error) return e.message;
    return String(e);
  }

  async function testMqtt() {
    if (!draft) return;
    testing = true;
    mqttTest = null;
    try {
      const text = await api.testMqtt($state.snapshot(draft.mqtt));
      mqttTest = { ok: true, text };
    } catch (e) {
      mqttTest = { ok: false, text: errorText(e) };
    } finally {
      testing = false;
    }
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
      <label><input type="checkbox" bind:checked={draft.general.restore_values} />
        Restore brightness and contrast on startup and after the computer wakes</label>
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
      </div>
      <div class="look">
        <div class="look-top">
          <div class="previews">
            <span class="tray-preview" style:color={draft.tray.brightness_color} style:font-family={draft.tray.font} title="Brightness">50</span>
            <span class="tray-preview" style:color={draft.tray.contrast_color} style:font-family={draft.tray.font} title="Contrast">50</span>
          </div>
          <select class="grow" bind:value={draft.tray.font} title="Default font">
            {#each fontOptions(draft.tray.font) as f (f)}<option value={f} style:font-family={f}>{f}</option>{/each}
          </select>
        </div>
        <div class="color-line">
          <label class="swatch" title="Brightness"><input type="color" bind:value={draft.tray.brightness_color} /> Brightness</label>
          <label class="swatch" title="Contrast"><input type="color" bind:value={draft.tray.contrast_color} /> Contrast</label>
          <label class="swatch" title="Volume"><input type="color" bind:value={draft.tray.volume_color} /> Volume</label>
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
        </div>
        <div class="look">
          <div class="look-top">
            <div class="previews">
              {#if mc.show_brightness_icon ?? true}
                {@const text = m.brightness == null ? "–" : String(m.brightness.current)}
                <span class="tray-preview" class:long={text.length > 2} title="Brightness icon"
                  style:color={mc.brightness_color || draft.tray.brightness_color}
                  style:font-family={mc.font || draft.tray.font}>{text}</span>
              {/if}
              {#if mc.show_contrast_icon ?? false}
                {@const text = m.contrast == null ? "–" : String(m.contrast.current)}
                <span class="tray-preview" class:long={text.length > 2} title="Contrast icon"
                  style:color={mc.contrast_color || draft.tray.contrast_color}
                  style:font-family={mc.font || draft.tray.font}>{text}</span>
              {/if}
            </div>
            <select class="grow" value={mc.font ?? ""} title="Icon font" onchange={(e) => (mc.font = e.currentTarget.value || null)}>
              <option value="">Default ({draft.tray.font})</option>
              {#each fontOptions(mc.font) as f (f)}<option value={f} style:font-family={f}>{f}</option>{/each}
            </select>
          </div>
          <div class="color-line">
            <label class="swatch" title="Brightness icon">
              <input type="color" value={mc.brightness_color ?? draft.tray.brightness_color}
                oninput={(e) => (mc.brightness_color = e.currentTarget.value)} />
              Brightness
            </label>
            <label class="swatch" title="Contrast icon">
              <input type="color" value={mc.contrast_color ?? draft.tray.contrast_color}
                oninput={(e) => (mc.contrast_color = e.currentTarget.value)} />
              Contrast
            </label>
            {#if mc.brightness_color || mc.contrast_color}
              <button class="reset" onclick={() => { mc.brightness_color = null; mc.contrast_color = null; }}>Reset</button>
            {/if}
          </div>
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
      <label><input type="checkbox" bind:checked={draft.mqtt.enabled} /> Enable MQTT</label>
      <label><input type="checkbox" bind:checked={draft.mqtt.discovery} /> Publish Home Assistant MQTT discovery</label>
      <p class="muted">Turn discovery off when using the lumenctl HACS integration. With discovery left on, Home Assistant creates a second copy of every entity.</p>
      <div class="grid">
        <span>Broker</span>
        <div class="row"><input type="text" class="grow" placeholder="192.168.2.95 or mqtt://192.168.2.95" bind:value={draft.mqtt.host} />
          <input type="number" class="port" bind:value={draft.mqtt.port} /></div>
        <span>Username</span><input type="text" value={draft.mqtt.username ?? ""} oninput={(e) => (draft!.mqtt.username = e.currentTarget.value || null)} />
        <span>Password</span><input type="password" value={draft.mqtt.password ?? ""} oninput={(e) => (draft!.mqtt.password = e.currentTarget.value || null)} />
        <span>Discovery prefix</span><input type="text" bind:value={draft.mqtt.discovery_prefix} />
        <span>Base topic</span><input type="text" bind:value={draft.mqtt.base_topic} />
        <span>Node id</span><input type="text" placeholder={info?.mqtt_node_id} value={draft.mqtt.node_id ?? ""}
          oninput={(e) => (draft!.mqtt.node_id = e.currentTarget.value || null)} />
      </div>
      <div class="row mqtt-test">
        <button disabled={testing} onclick={testMqtt}>{testing ? "Testing…" : "Test connection"}</button>
        {#if mqttTest}<span class:ok={mqttTest.ok} class:bad={!mqttTest.ok}>{mqttTest.text}</span>{/if}
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
    min-width: 0;
  }
  .form > * {
    min-width: 0;
    max-width: 100%;
  }
  .form label {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 8.5rem) minmax(0, 1fr);
    gap: 6px 10px;
    align-items: center;
  }
  .grid > * {
    min-width: 0;
  }
  .grid > span {
    color: var(--text-2);
  }
  .grid select,
  .grid input[type="text"],
  .grid input[type="password"],
  .look-fields select,
  .row input[type="text"] {
    width: 100%;
    min-width: 0;
    max-width: 100%;
  }
  .grid input[type="number"] {
    width: 4.2rem;
    min-width: 0;
  }
  .row {
    flex-wrap: wrap;
    min-width: 0;
  }
  .look {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .look-top {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .look-top select {
    flex: 1;
    width: 0;
    min-width: 0;
  }
  .previews {
    display: flex;
    gap: 4px;
    flex: none;
  }
  .tray-preview {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 7px;
    background: #1a1a1a;
    color: var(--brightness);
    font-weight: 700;
    font-size: 15px;
    line-height: 1;
    letter-spacing: -0.04em;
  }
  .tray-preview.long {
    font-size: 11px;
  }
  .color-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
  }
  .swatch {
    color: var(--text-2);
    font-size: 12px;
    flex: none;
  }
  .reset {
    padding: 2px 8px;
    font-size: 12px;
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
    width: auto;
    flex: 1 1 6rem;
    min-width: 0;
  }
  .mono,
  code {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
  }
  p {
    margin: 0;
  }
  .mqtt-test {
    margin-top: 8px;
    flex-wrap: wrap;
  }
  .mqtt-test span {
    font-size: 12px;
    user-select: text;
  }
  .mqtt-test .ok { color: #0b7a32; }
  .mqtt-test .bad { color: var(--danger); }
  @media (prefers-color-scheme: dark) {
    .mqtt-test .ok { color: #3ddc84; }
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
