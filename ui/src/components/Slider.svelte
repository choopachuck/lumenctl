<script lang="ts">
  import Icon from "./Icon.svelte";

  let {
    label,
    icon,
    value,
    max = 100,
    color = "var(--accent)",
    step = 5,
    disabled = false,
    onchange,
  }: {
    label: string;
    icon?: string;
    value: number | null | undefined;
    max?: number;
    color?: string;
    step?: number;
    disabled?: boolean;
    onchange: (v: number) => void;
  } = $props();

  let dragging = $state(false);
  let local = $state(0);
  // While dragging, keep the thumb under the pointer and ignore echoes from the backend.
  let shown = $derived(dragging ? local : (value ?? 0));
  let pct = $derived(max > 0 ? (shown / max) * 100 : 0);
  let frame = 0;

  function emit(v: number) {
    local = v;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => onchange(v));
  }

  function wheel(e: WheelEvent) {
    if (disabled || value == null) return;
    e.preventDefault();
    const dir = e.deltaY < 0 ? 1 : -1;
    const next = Math.max(0, Math.min(max, (dragging ? local : value) + dir * step));
    onchange(next);
  }
</script>

<div class="slider" class:disabled={disabled || value == null} style:--c={color} onwheel={wheel}>
  <div class="head">
    {#if icon}<span class="ico"><Icon name={icon} size={15} /></span>{/if}
    <span class="label">{label}</span>
    <span class="value">{value == null ? "—" : shown}</span>
  </div>
  <input
    type="range"
    min="0"
    {max}
    value={shown}
    disabled={disabled || value == null}
    style:--p="{pct}%"
    onpointerdown={() => { dragging = true; local = value ?? 0; }}
    onpointerup={() => (dragging = false)}
    onpointercancel={() => (dragging = false)}
    oninput={(e) => emit(+e.currentTarget.value)}
    onkeydown={() => (local = value ?? 0)}
  />
</div>

<style>
  .slider {
    padding: 4px 0;
  }
  .slider.disabled {
    opacity: 0.45;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
  }
  .ico {
    color: var(--c);
    display: flex;
  }
  .label {
    flex: 1;
    color: var(--text-2);
  }
  .value {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    min-width: 2.5em;
    text-align: right;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 20px;
    background: transparent;
    margin: 0;
  }
  input[type="range"]::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--c) var(--p), var(--track) var(--p));
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 18px;
    height: 18px;
    margin-top: -7px;
    border-radius: 50%;
    background: var(--c);
    border: 4px solid color-mix(in srgb, Canvas 85%, var(--c));
    box-shadow: 0 0 0 1px var(--border);
    transition: transform 0.1s;
  }
  input[type="range"]:active::-webkit-slider-thumb {
    transform: scale(0.85);
  }
</style>
