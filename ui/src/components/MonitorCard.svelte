<script lang="ts">
  import { api, type Monitor, type NamedValue } from "../lib/api";
  import { app, patch, saveConfig, toast } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";
  import Slider from "./Slider.svelte";

  let { monitor }: { monitor: Monitor } = $props();

  let expanded = $state(false);
  let renaming = $state(false);
  let alias = $state("");

  let on = $derived(monitor.power == null || monitor.power === "on");
  let step = $derived(app.config?.tray.wheel_step_brightness ?? 5);
  let subtitle = $derived(
    [monitor.output, monitor.alias ? monitor.name : null, monitor.serial ? `SN ${monitor.serial}` : null]
      .filter(Boolean)
      .join(" · "),
  );

  function startRename() {
    alias = monitor.alias ?? "";
    renaming = true;
  }

  async function commitRename() {
    renaming = false;
    const value = alias.trim() || null;
    if (value === monitor.alias) return;
    await saveConfig((c) => {
      let mc = c.monitor.find((m) => m.id === monitor.id);
      if (!mc) {
        mc = {
          id: monitor.id, alias: null, ignore: false, use_6b: false, show_brightness_icon: null,
          show_contrast_icon: null, min_brightness: null, max_brightness: null, min_contrast: null,
          max_contrast: null, font: null,
        };
        c.monitor.push(mc);
      }
      mc.alias = value;
    });
  }

  async function action(a: "factory_reset" | "rotate" | "power_toggle") {
    if (a === "factory_reset" && !confirm(`Restore factory settings on ${monitor.display_name}?`)) return;
    try {
      await api.action(monitor.key, a);
    } catch (e) {
      toast(String(e));
    }
  }

  function choose(list: NamedValue[], current: NamedValue | null) {
    if (!current || list.some((x) => x.value === current.value)) return list;
    return [current, ...list];
  }
</script>

<section class="card" class:off={!on}>
  <header class="row">
    <div class="badge">{monitor.index}</div>
    <div class="grow title">
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={alias} autofocus placeholder={monitor.name}
          onblur={commitRename} onkeydown={(e) => e.key === "Enter" && commitRename()} />
      {:else}
        <div class="name" role="button" tabindex="0" ondblclick={startRename}
          onkeydown={(e) => e.key === "F2" && startRename()} title="Double-click or F2 to rename">{monitor.display_name}</div>
        <div class="muted small">{subtitle || monitor.description}</div>
      {/if}
    </div>
    <button class="icon" class:active={on} title={on ? "Power off" : "Power on"}
      onclick={() => patch(monitor, { power: !on })}>
      <Icon name="power" />
    </button>
    <button class="icon chev" class:open={expanded} title="More" onclick={() => (expanded = !expanded)}>
      <Icon name="chevron" />
    </button>
  </header>

  {#if !monitor.ddc_ok && monitor.brightness == null}
    <p class="muted small warn">{monitor.last_error ? "DDC/CI is not responding" : "Reading monitor…"}</p>
  {/if}

  <Slider label="Brightness" icon="sun" color="var(--brightness)" {step}
    value={monitor.brightness?.current} max={monitor.brightness?.max || 100}
    onchange={(v) => patch(monitor, { brightness: v })} />
  <Slider label="Contrast" icon="contrast" color="var(--contrast)" step={app.config?.tray.wheel_step_contrast ?? 10}
    value={monitor.contrast?.current} max={monitor.contrast?.max || 100}
    onchange={(v) => patch(monitor, { contrast: v })} />

  {#if expanded}
    <div class="more">
      {#if monitor.volume}
        <Slider label="Speaker volume" icon="volume" color="var(--volume)"
          value={monitor.volume.current} max={monitor.volume.max || 100}
          onchange={(v) => patch(monitor, { volume: v })} />
      {/if}
      {#if monitor.red || monitor.green || monitor.blue}
        <Slider label="Red" color="var(--red)" value={monitor.red?.current} max={monitor.red?.max || 100}
          onchange={(v) => patch(monitor, { red: v })} />
        <Slider label="Green" color="var(--green)" value={monitor.green?.current} max={monitor.green?.max || 100}
          onchange={(v) => patch(monitor, { green: v })} />
        <Slider label="Blue" color="var(--blue)" value={monitor.blue?.current} max={monitor.blue?.max || 100}
          onchange={(v) => patch(monitor, { blue: v })} />
      {/if}

      <div class="selects">
        {#if monitor.input}
          <label><Icon name="input" size={14} /> Input</label>
          <select value={monitor.input.value}
            onchange={(e) => confirm("Switch input? The picture may disappear from this computer.")
              ? patch(monitor, { input: +e.currentTarget.value })
              : (e.currentTarget.value = String(monitor.input?.value))}>
            {#each choose(monitor.supported_inputs, monitor.input) as i (i.value)}
              <option value={i.value}>{i.name}</option>
            {/each}
          </select>
        {/if}
        {#if monitor.color_preset}
          <label><Icon name="palette" size={14} /> Color</label>
          <select value={monitor.color_preset.value}
            onchange={(e) => patch(monitor, { color_preset: +e.currentTarget.value })}>
            {#each choose(monitor.supported_color_presets, monitor.color_preset) as p (p.value)}
              <option value={p.value}>{p.name}</option>
            {/each}
          </select>
        {/if}
        {#if monitor.display_mode}
          <label><Icon name="chip" size={14} /> Mode</label>
          <select value={monitor.display_mode.value}
            onchange={(e) => patch(monitor, { display_mode: +e.currentTarget.value })}>
            {#each choose(monitor.supported_display_modes, monitor.display_mode) as p (p.value)}
              <option value={p.value}>{p.name}</option>
            {/each}
          </select>
        {/if}
      </div>

      <div class="row actions">
        <button onclick={() => api.refresh(monitor.key)}><Icon name="refresh" size={14} /> Re-read</button>
        <button onclick={() => action("rotate")}><Icon name="rotate" size={14} /> Rotate</button>
        <button onclick={() => { app.vcpMonitor = monitor.key; app.view = "vcp"; }}>
          <Icon name="chip" size={14} /> VCP
        </button>
        <span class="grow"></span>
        <button class="danger" onclick={() => action("factory_reset")}>Reset</button>
      </div>
      <div class="muted small id" title="Monitor id / API key">{monitor.key}</div>
    </div>
  {/if}
</section>

<style>
  .card.off {
    opacity: 0.6;
  }
  header {
    margin-bottom: 6px;
  }
  .badge {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font-weight: 600;
    background: var(--track);
  }
  .title .name {
    font-weight: 600;
    font-size: 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .title input {
    width: 100%;
  }
  .small {
    font-size: 11.5px;
  }
  .warn {
    margin: 2px 0 4px;
  }
  .icon.active {
    color: var(--accent);
  }
  .chev :global(svg) {
    transition: transform 0.15s;
  }
  .chev.open :global(svg) {
    transform: rotate(180deg);
  }
  .more {
    border-top: 1px solid var(--border);
    margin-top: 6px;
    padding-top: 6px;
  }
  .selects {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
    margin: 8px 0;
  }
  .selects label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .actions {
    flex-wrap: wrap;
    gap: 6px;
  }
  .id {
    margin-top: 8px;
    user-select: text;
    word-break: break-all;
  }
</style>
