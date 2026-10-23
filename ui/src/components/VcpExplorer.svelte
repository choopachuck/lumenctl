<script lang="ts">
  import { api, hex, VCP_NAMES, type Capabilities, type VcpValue } from "../lib/api";
  import { app, toast } from "../lib/store.svelte";

  let reference = $state(app.vcpMonitor ?? app.monitors[0]?.key ?? "");
  let caps = $state<Capabilities | null>(null);
  let loading = $state(false);
  let values = $state<Record<number, VcpValue | string>>({});
  let custom = $state("");

  async function load(refresh = false) {
    if (!reference) return;
    loading = true;
    values = {};
    try {
      caps = await api.capabilities(reference, refresh);
    } catch (e) {
      caps = null;
      toast(String(e));
    } finally {
      loading = false;
    }
  }

  async function read(code: number) {
    try {
      values[code] = await api.readVcp(reference, code);
    } catch (e) {
      values[code] = String(e);
    }
  }

  async function write(code: number, raw: string) {
    if (!raw.trim()) return;
    if (!confirm(`Write ${raw} to VCP ${hex(code)}? Unknown codes can misconfigure the monitor.`)) return;
    try {
      await api.writeVcp(reference, code, /^[+-]/.test(raw) ? raw : +raw);
      await read(code);
    } catch (e) {
      toast(String(e));
    }
  }

  $effect(() => {
    reference;
    load();
  });

  let codes = $derived(caps ? Object.keys(caps.vcp).map(Number).sort((a, b) => a - b) : []);
</script>

<div class="scroll">
  <div class="row top">
    <select class="grow" bind:value={reference}>
      {#each app.monitors as m (m.key)}<option value={m.key}>{m.index}. {m.display_name}</option>{/each}
    </select>
    <button onclick={() => load(true)} disabled={loading}>{loading ? "Reading…" : "Reload"}</button>
  </div>
  <p class="muted">Capability strings are often incomplete; codes not listed here may still work. Writing unknown
    codes can misconfigure the monitor.</p>

  {#if caps}
    <div class="card meta">
      <div><span class="muted">Model</span> {caps.model ?? "—"}</div>
      <div><span class="muted">MCCS</span> {caps.mccs_version ?? "—"}</div>
    </div>
    <div class="card list">
      {#each codes as code (code)}
        {@const v = values[code]}
        <div class="code">
          <div class="row">
            <code>{hex(code)}</code>
            <span class="grow">{VCP_NAMES[code] ?? "Unknown"}</span>
            <button onclick={() => read(code)}>Read</button>
          </div>
          {#if caps.vcp[code].length}
            <div class="muted small">values: {caps.vcp[code].map(hex).join(" ")}</div>
          {/if}
          {#if v !== undefined}
            <div class="row small">
              {#if typeof v === "string"}<span class="err">{v}</span>
              {:else}<span>current <b>{v.current}</b> / max {v.max}</span>{/if}
              <span class="grow"></span>
              <input type="text" class="val" placeholder="value"
                onkeydown={(e) => e.key === "Enter" && write(code, e.currentTarget.value)} />
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <h2>Any code</h2>
  <div class="card row">
    <input type="text" class="grow" placeholder="0x10" bind:value={custom} />
    <button onclick={() => { const c = parseInt(custom, custom.startsWith("0x") ? 16 : 10); if (c >= 0 && c < 256) read(c); }}>Read</button>
  </div>
  {#each Object.entries(values).filter(([c]) => !codes.includes(+c)) as [c, v]}
    <div class="card small"><code>{hex(+c)}</code> {typeof v === "string" ? v : `current ${v.current} / max ${v.max}`}</div>
  {/each}

  {#if caps}<details><summary class="muted">Raw capability string</summary><pre>{caps.raw}</pre></details>{/if}
</div>

<style>
  .top {
    margin: 4px 0 8px;
  }
  p {
    margin: 0 0 8px;
    font-size: 12px;
  }
  .meta {
    display: flex;
    gap: 20px;
  }
  .list {
    padding: 4px 10px;
  }
  .code {
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .code:last-child {
    border: none;
  }
  code,
  pre {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
  }
  pre {
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .small {
    font-size: 11.5px;
    margin-top: 4px;
  }
  .val {
    width: 80px;
    padding: 2px 6px;
  }
  .err {
    color: var(--danger);
  }
</style>
