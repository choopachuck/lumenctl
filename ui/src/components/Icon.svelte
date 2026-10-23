<script lang="ts">
  const paths: Record<string, string> = {
    sun: "M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10Zm0-5v2m0 16v2M4.2 4.2l1.4 1.4m12.8 12.8 1.4 1.4M2 12h2m16 0h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4",
    contrast: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 0v18",
    volume: "M4 9v6h4l5 4V5L8 9H4Zm12 .5a4 4 0 0 1 0 5M18.5 7a7.5 7.5 0 0 1 0 10",
    power: "M12 3v8m5.7-4.7a8 8 0 1 1-11.4 0",
    settings:
      "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Zm7.4 4.5.1-1.5-.1-1.5 2-1.6-2-3.4-2.4.9a7 7 0 0 0-2.6-1.5L14 2.5h-4l-.4 2.4A7 7 0 0 0 7 6.4l-2.4-.9-2 3.4 2 1.6-.1 1.5.1 1.5-2 1.6 2 3.4 2.4-.9a7 7 0 0 0 2.6 1.5l.4 2.4h4l.4-2.4a7 7 0 0 0 2.6-1.5l2.4.9 2-3.4-2-1.6Z",
    refresh: "M20 11a8 8 0 0 0-14.6-4.5M4 4v3h3m-3 6a8 8 0 0 0 14.6 4.5M20 20v-3h-3",
    terminal: "M4 5h16v14H4zM7 9l3 3-3 3m5 0h5",
    chevron: "m6 9 6 6 6-6",
    back: "m15 6-6 6 6 6",
    close: "M6 6l12 12M18 6 6 18",
    link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
    rotate: "M4 12a8 8 0 1 0 2.3-5.7M4 4v4h4",
    chip: "M7 7h10v10H7zM9 3v4m6-4v4M9 17v4m6-4v4M3 9h4m-4 6h4m10-6h4m-4 6h4",
    input: "M3 5h18v11H3zM8 20h8m-4-4v4",
    palette: "M12 3a9 9 0 0 0 0 18c1 0 1.5-.7 1.5-1.5 0-1.2-1-1.5-1-2.5s.8-1.5 2-1.5H17a4 4 0 0 0 4-4c0-4.7-4-8.5-9-8.5ZM7.5 11.5h.01M10 7.5h.01M15 7.5h.01",
    plus: "M12 5v14M5 12h14",
    trash: "M5 7h14M10 11v6m4-6v6M6 7l1 13h10l1-13M9 7V4h6v3",
  };
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"
  stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d={paths[name] ?? ""} />
</svg>
