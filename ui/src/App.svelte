<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import { app, init, toast } from "./lib/store.svelte";
  import Console from "./components/Console.svelte";
  import Icon from "./components/Icon.svelte";
  import MonitorCard from "./components/MonitorCard.svelte";
  import Settings from "./components/Settings.svelte";
  import VcpExplorer from "./components/VcpExplorer.svelte";

  let scanning = $state(false);
  const titles = { monitors: "lumenctl", settings: "Settings", console: "Commands", vcp: "VCP codes" };

  onMount(() => {
    init().catch((e) => toast(String(e)));
    const esc = (e: KeyboardEvent) => e.key === "Escape" && getCurrentWindow().hide();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  });

  async function rescan() {
    scanning = true;
    try {
      app.monitors = await api.rescan();
    } catch (e) {
      toast(String(e));
    } finally {
      scanning = false;
    }
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  {#if app.view !== "monitors"}
    <button class="icon" title="Back" onclick={() => (app.view = "monitors")}><Icon name="back" /></button>
  {/if}
  <span class="title" data-tauri-drag-region>{titles[app.view]}</span>
  {#if app.view === "monitors"}
    <button class="icon" class:on={app.linkAll} title="Change all monitors together"
      onclick={() => (app.linkAll = !app.linkAll)}><Icon name="link" /></button>
    <button class="icon" class:spin={scanning} title="Rescan monitors" onclick={rescan}><Icon name="refresh" /></button>
    <button class="icon" title="Commands" onclick={() => (app.view = "console")}><Icon name="terminal" /></button>
    <button class="icon" title="Settings" onclick={() => (app.view = "settings")}><Icon name="settings" /></button>
  {/if}
  <button class="icon" title="Hide (Esc)" onclick={() => getCurrentWindow().hide()}><Icon name="close" /></button>
</header>

{#if app.view === "monitors"}
  <main class="scroll">
    {#each app.monitors as monitor (monitor.id)}
      <MonitorCard {monitor} />
    {:else}
      <div class="empty muted">
        <p>No DDC/CI monitors found.</p>
        <p>Make sure DDC/CI is enabled in the monitor's on-screen menu.</p>
        <button onclick={rescan}>Rescan</button>
      </div>
    {/each}
  </main>
{:else if app.view === "settings"}
  <Settings />
{:else if app.view === "console"}
  <Console />
{:else if app.view === "vcp"}
  <VcpExplorer />
{/if}

{#if app.toast}
  <div class="toast">{app.toast}</div>
{/if}

<style>
  .titlebar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 8px 6px 14px;
    flex-shrink: 0;
  }
  .title {
    flex: 1;
    font-weight: 600;
    font-size: 13px;
  }
  .titlebar :global(button.on) {
    color: var(--accent);
    background: var(--surface-hover);
  }
  .spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .empty {
    text-align: center;
    padding: 40px 20px;
  }
  .toast {
    position: fixed;
    left: 12px;
    right: 12px;
    bottom: 12px;
    padding: 10px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, Canvas 92%, var(--accent));
    border: 1px solid var(--border);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    white-space: pre-wrap;
  }
</style>
