<script lang="ts">
  // The Night sky theme's background: pixel stars that twinkle, now and then a falling
  // star, and a bright one when the connection comes up. Plain elements with CSS
  // animations, which stop while the window is hidden; "reduce motion" keeps it still.
  import { onDestroy } from 'svelte';
  import { daysUntil } from '../lib/format';
  import { app } from '../lib/state.svelte';

  // The same sky on every start: a small seeded generator instead of Math.random.
  function rng(seed: number) {
    return () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  const r = rng(20261028);
  const stars = Array.from({ length: 120 }, (_, i) => {
    // Every third star is a faint one in a band across the sky, like the Milky Way.
    const band = i % 3 === 0;
    const t = r();
    const big = !band && r() < 0.06;
    return {
      x: band ? t * 100 : r() * 100,
      y: band ? 4 + t * 78 + (r() - 0.5) * 16 : r() * 100,
      size: band ? 1 : big || r() < 0.3 ? 2 : 1,
      big,
      color: band ? '#7c7c88' : r() < 0.6 ? '#ffffff' : '#bdbdc8',
      dur: 2.5 + r() * 5,
      delay: -r() * 8,
    };
  });
  const falls = Array.from({ length: 4 }, (_, i) => ({
    x: 30 + r() * 70,
    y: -6 + r() * 34,
    dur: 10 + r() * 8,
    delay: 0.6 + i * 2.6 + r() * 1.2,
    len: Math.round(90 + r() * 100),
  }));

  // The moon shows what is left of the subscription: full with plenty of time and
  // traffic, a thinning crescent near the end, only its outline once it is used up.
  let left = $derived.by(() => {
    const u = app.activeProfile?.usage;
    if (!u) return 1;
    let f = 1;
    if (u.expire) f = Math.min(f, Math.max(0, daysUntil(u.expire)) / 30);
    if (u.total) f = Math.min(f, Math.max(0, 1 - (u.upload + u.download) / u.total));
    return f;
  });
  let moon = $derived(moonCells(left));

  /** 16×16 pixels: the lit disc minus a shadow disc that slides off to the right as `f`
   *  (what is left, 0–1) grows. */
  function moonCells(f: number) {
    const r = 7, c = 7.5, shift = f * (2 * r + 1);
    let lit = '', rim = '';
    for (let y = 0; y < 16; y++) {
      for (let x = 0; x < 16; x++) {
        const dist = Math.hypot(x + 0.5 - c, y + 0.5 - c);
        if (dist > r - 0.2) continue; // trims the single-pixel tips
        const cell = `M${x} ${y}h1v1H${x}z`;
        if (Math.hypot(x + 0.5 - c - shift, y + 0.5 - c) > r) lit += cell;
        else if (dist > r - 1.4) rim += cell;
      }
    }
    return { lit, rim };
  }

  let wish = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let was = app.status.running;
  $effect(() => {
    const running = app.status.running;
    if (running && !was) {
      wish = true;
      clearTimeout(timer);
      timer = setTimeout(() => (wish = false), 1700);
    }
    was = running;
  });
  onDestroy(() => clearTimeout(timer));
</script>

<div class="sky" aria-hidden="true">
  {#each stars as s, i (i)}
    <i class="star" class:big={s.big} style:left="{s.x}%" style:top="{s.y}%" style:width="{s.size}px" style:height="{s.size}px"
      style:background={s.color} style:animation-duration="{s.dur}s" style:animation-delay="{s.delay}s"></i>
  {/each}
  <svg class="moon" viewBox="0 0 16 16" shape-rendering="crispEdges"><path class="rim" d={moon.rim} /><path d={moon.lit} /></svg>
  {#each falls as f, i (i)}
    <div class="fall" style:left="{f.x}%" style:top="{f.y}%" style:animation-duration="{f.dur}s" style:animation-delay="{f.delay}s"><span style:width="{f.len}px"></span></div>
  {/each}
  {#if wish}<div class="fall wish"><span></span></div>{/if}
</div>

<style>
  .sky { position: fixed; inset: 0; z-index: 0; overflow: hidden; pointer-events: none; background: #000; }
  .star { position: absolute; display: block; animation: twinkle 4s steps(3, jump-none) infinite; }
  .star.big { box-shadow: 2px 0 rgb(255 255 255 / .5), -2px 0 rgb(255 255 255 / .5), 0 2px rgb(255 255 255 / .5), 0 -2px rgb(255 255 255 / .5); }
  @keyframes twinkle { 0%, 100% { opacity: .3; } 50% { opacity: 1; } }
  .moon { position: absolute; top: 64px; right: 40px; width: 48px; height: 48px; fill: #f1f1f4; opacity: .9; }
  .moon .rim { fill: #3a3a42; }
  /* A falling star: a head pixel and a stepped tail, crossing in the first tenth of its cycle. */
  .fall { position: absolute; width: 0; height: 0; opacity: 0; animation: fall 12s linear infinite; }
  .fall span { position: absolute; left: 0; top: -1px; height: 2px; transform: rotate(-38.8deg); transform-origin: 0 50%;
    background: linear-gradient(90deg, #fff 0 6px, rgb(255 255 255 / .7) 6px 16px, rgb(255 255 255 / .4) 16px 34px, rgb(255 255 255 / .18) 34px 64px, rgb(255 255 255 / .06) 64px 100%); }
  .fall span::before { content: ''; position: absolute; left: -1px; top: -1px; width: 4px; height: 4px; background: #fff; }
  @keyframes fall { 0% { transform: translate(0, 0); opacity: 0; } 1% { opacity: 1; } 9%, 100% { transform: translate(-560px, 450px); opacity: 0; } }
  .wish { left: 72%; top: 3%; animation: wish 1.6s linear forwards; }
  .wish span { width: 260px; }
  @keyframes wish { 0% { transform: translate(0, 0); opacity: 0; } 5% { opacity: 1; } 100% { transform: translate(-900px, 723px); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) {
    .star { animation: none; }
    .fall { display: none; }
  }
</style>
