<script lang="ts">
  import { fmtSpeed } from '../lib/format';

  let { down, up }: { down: number[]; up: number[] } = $props();
  let canvas: HTMLCanvasElement;
  let width = $state(0);

  const STEPS = [64, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072].map((k) => k * 1024);

  $effect(() => {
    void down; void up; void width;
    draw();
  });

  function draw() {
    if (!canvas || !width) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth, h = canvas.clientHeight;
    canvas.width = w * dpr; canvas.height = h * dpr;
    const ctx = canvas.getContext('2d')!;
    ctx.scale(dpr, dpr);
    const cs = getComputedStyle(document.documentElement);
    const col = (v: string) => cs.getPropertyValue(v).trim();
    const n = down.length;
    const peak = Math.max(...down, ...up, 1);
    const max = STEPS.find((s) => s >= peak * 1.08) ?? STEPS[STEPS.length - 1];
    const L = 68, R = 6, T = 8, B = 18, pw = w - L - R, ph = h - T - B;
    ctx.font = `11px ${col('--f-mono')}`;
    ctx.textBaseline = 'middle';
    for (let i = 0; i <= 4; i++) {
      const y = T + ph - (ph * i) / 4;
      ctx.strokeStyle = col('--line'); ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(L, Math.round(y) + 0.5); ctx.lineTo(w - R, Math.round(y) + 0.5); ctx.stroke();
      ctx.fillStyle = col('--faint'); ctx.textAlign = 'right';
      ctx.fillText(i ? fmtSpeed((max * i) / 4).replace('.00', '') : '0', L - 8, y);
    }
    ctx.textAlign = 'left'; ctx.fillText(`${n} s ago`, L, h - 6);
    ctx.textAlign = 'right'; ctx.fillText('now', w - R, h - 6);
    const X = (i: number) => L + (pw * i) / (n - 1);
    const Y = (v: number) => T + ph - (ph * v) / max;
    const series = (arr: number[], color: string) => {
      ctx.beginPath();
      arr.forEach((v, i) => (i ? ctx.lineTo(X(i), Y(v)) : ctx.moveTo(X(i), Y(v))));
      ctx.strokeStyle = color; ctx.lineWidth = 1.75; ctx.lineJoin = 'round'; ctx.stroke();
      ctx.lineTo(X(n - 1), T + ph); ctx.lineTo(X(0), T + ph); ctx.closePath();
      ctx.globalAlpha = 0.12; ctx.fillStyle = color; ctx.fill(); ctx.globalAlpha = 1;
      ctx.beginPath(); ctx.arc(X(n - 1), Y(arr[n - 1]), 3.2, 0, 7); ctx.fillStyle = color; ctx.fill();
    };
    series(down, col('--accent'));
    series(up, col('--upload'));
  }
</script>

<div bind:clientWidth={width} role="img" aria-label="Download and upload speed over the last minute">
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  canvas { width: 100%; height: 190px; display: block; }
</style>
