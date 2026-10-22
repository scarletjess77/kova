<script lang="ts" module>
  export const ICONS = {
    dashboard: 'M3 13h8V3H3zM13 21h8V11h-8zM3 21h8v-5H3zM13 3v5h8V3z',
    proxies: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9M12 3C9.5 5.6 8.2 8.6 8.2 12s1.3 6.4 3.8 9',
    profiles: 'M12 3l9 5-9 5-9-5zM3 12.5l9 5 9-5M3 17l9 5 9-5',
    connections: 'M7 7h13M16 3l4 4-4 4M17 17H4M8 13l-4 4 4 4',
    rules: 'M4 6h16M7 12h10M10 18h4',
    menu: 'M4 7h16M4 12h16M4 17h16',
    logs: 'M4 5h16v14H4zM8 10l3 2-3 2M13 15h4',
    settings: 'M4 7h9M17 7h3M4 17h3M11 17h9M15 5v4M9 15v4',
    power: 'M12 3v8M6.3 7.3a8 8 0 1 0 11.4 0',
    search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
    bolt: 'M13 2 4 14h7l-1 8 9-12h-7z',
    plus: 'M12 5v14M5 12h14',
    refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7',
    x: 'M6 6l12 12M18 6 6 18',
    clip: 'M9 3h6v3H9zM7 4.5H5V21h14V4.5h-2',
    file: 'M6 3h8l5 5v13H6zM14 3v5h5',
    link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
    pause: 'M8 5v14M16 5v14',
    play: 'M7 4l13 8-13 8z',
    shield: 'M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z',
    monitor: 'M3 4h18v12H3zM8 20h8M12 16v4',
    edit: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
    check: 'M5 12l5 5 9-10',
    copy: 'M8 8h12v12H8zM4 16V4h12',
    moon: 'M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z',
    min: 'M5 12h14',
    max: 'M6 6h12v12H6z',
    trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13',
    down: 'M12 5v14M6 13l6 6 6-6',
    globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18',
    group: 'M13 2 4 14h7l-1 8 9-12h-7z',
    alert: 'M12 3l9 16H3zM12 10v4M12 17v.5',
    external: 'M14 4h6v6M20 4l-9 9M18 14v6H4V6h6',
    folder: 'M3 6h6l2 2h10v11H3z',
    grip: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
    pulse: 'M3 12h4l3-8 4 16 3-8h4',
    qr: 'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h2v2h-2zM18 14h2v2h-2zM14 18h2v2h-2zM18 18h2v2h-2z',
    heart: 'M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z',
  } as const;
  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, class: cls = '' }: { name: IconName; class?: string } = $props();
</script>

<svg class="i {cls}" viewBox="0 0 24 24" aria-hidden="true"><path d={ICONS[name]} /></svg>
