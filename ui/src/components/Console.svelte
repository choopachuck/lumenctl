<script lang="ts">
  import { api } from "../lib/api";

  type Line = { kind: "in" | "out" | "err"; text: string };
  let input = $state("");
  let lines = $state<Line[]>([]);
  let history: string[] = [];
  let pos = -1;
  let log: HTMLDivElement;

  const examples = [
    ["b 50", "brightness 50 on the default monitor"],
    ["MateView b-10 N2724A b+10", "per monitor, relative"],
    ["t b 0 c 0 t b 90 c 80", "toggle between two profiles"],
    ["2 s HDMI1", "monitor #2 → input HDMI1"],
    ["p 6500K  j Movie", "color preset / display mode"],
    ["v + 5  m  u", "system volume / mute / unmute"],
    ["x", "all monitors to power-save"],
    ["getVCP 0x12", "read a raw VCP code"],
  ];

  async function run() {
    const cmd = input.trim();
    if (!cmd) return;
    history.unshift(cmd);
    pos = -1;
    input = "";
    lines.push({ kind: "in", text: cmd });
    try {
      const out = await api.execute(cmd);
      for (const m of out.messages) lines.push({ kind: "out", text: m });
      if (out.value != null) lines.push({ kind: "out", text: `→ ${out.value}` });
    } catch (e) {
      lines.push({ kind: "err", text: String(e) });
    }
    queueMicrotask(() => log?.scrollTo({ top: log.scrollHeight }));
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Enter") run();
    else if (e.key === "ArrowUp" && history.length) {
      pos = Math.min(pos + 1, history.length - 1);
      input = history[pos];
      e.preventDefault();
    } else if (e.key === "ArrowDown") {
      pos = Math.max(pos - 1, -1);
      input = pos < 0 ? "" : history[pos];
      e.preventDefault();
    }
  }
</script>

<div class="scroll" bind:this={log}>
  {#if lines.length === 0}
    <p class="muted">ClickMonitorDDC-compatible command language. Also available via
      <code>lumenctl.exe &lt;command&gt;</code>, <code>POST /api/command</code> and MQTT.</p>
    <div class="card examples">
      {#each examples as [cmd, desc]}
        <button class="ex" onclick={() => (input = cmd)}><code>{cmd}</code><span class="muted">{desc}</span></button>
      {/each}
    </div>
  {/if}
  {#each lines as l}
    <div class="line {l.kind}">{l.kind === "in" ? "› " : ""}{l.text}</div>
  {/each}
</div>
<footer>
  <!-- svelte-ignore a11y_autofocus -->
  <input type="text" class="mono" placeholder="Type a command, e.g. b 40 c 50" bind:value={input} onkeydown={key} autofocus />
</footer>

<style>
  code,
  .mono,
  .line {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
  }
  .examples {
    display: flex;
    flex-direction: column;
    padding: 4px;
  }
  .ex {
    border: none;
    background: none;
    justify-content: space-between;
    text-align: left;
    gap: 12px;
  }
  .ex:hover {
    background: var(--surface-hover);
  }
  .line {
    white-space: pre-wrap;
    padding: 2px 0;
    user-select: text;
  }
  .line.in {
    color: var(--accent);
  }
  .line.err {
    color: var(--danger);
  }
  footer {
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    display: flex;
  }
  footer input {
    flex: 1;
  }
</style>
