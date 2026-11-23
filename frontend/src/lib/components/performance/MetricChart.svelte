<script lang="ts">
  /**
   * Responsive SVG area chart for one metric over time. Gaps (null values or
   * jumps longer than `gapSecs`, i.e. the server was stopped) break the line
   * instead of drawing a misleading slope. Hover shows the nearest sample.
   */
  let { points, label, unit = '', color = 'teal', max = null, digits = 1, gapSecs = 10 }: {
    points: { t: number; v: number | null }[];
    label: string;
    unit?: string;
    color?: 'teal' | 'bedrock' | 'yellow' | 'red';
    max?: number | null;
    digits?: number;
    gapSecs?: number;
  } = $props();

  const W = 600;
  const H = 140;
  let hover = $state<number | null>(null);
  let svg = $state<SVGSVGElement | null>(null);

  const valid = $derived(points.filter((p) => p.v !== null) as { t: number; v: number }[]);
  const t0 = $derived(points.length ? points[0].t : 0);
  const t1 = $derived(points.length ? points[points.length - 1].t : 1);
  const top = $derived(max ?? Math.max(1, ...valid.map((p) => p.v)) * 1.15);
  const x = (t: number) => (t1 > t0 ? ((t - t0) / (t1 - t0)) * W : W);
  const y = (v: number) => H - Math.min(1, Math.max(0, v / top)) * H;

  /** Path segments split at gaps. */
  const segments = $derived.by(() => {
    const out: { t: number; v: number }[][] = [];
    let cur: { t: number; v: number }[] = [];
    let prev: number | null = null;
    for (const p of points) {
      if (p.v === null || (prev !== null && p.t - prev > gapSecs)) {
        if (cur.length) out.push(cur);
        cur = [];
      }
      if (p.v !== null) cur.push({ t: p.t, v: p.v });
      prev = p.t;
    }
    if (cur.length) out.push(cur);
    return out;
  });
  const line = (seg: { t: number; v: number }[]) => seg.map((p, i) => `${i ? 'L' : 'M'}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join('');
  const area = (seg: { t: number; v: number }[]) =>
    `${line(seg)}L${x(seg[seg.length - 1].t).toFixed(1)},${H}L${x(seg[0].t).toFixed(1)},${H}Z`;

  const latest = $derived(valid.length ? valid[valid.length - 1].v : null);
  const avg = $derived(valid.length ? valid.reduce((a, p) => a + p.v, 0) / valid.length : null);
  const peak = $derived(valid.length ? Math.max(...valid.map((p) => p.v)) : null);
  const fmt = (v: number | null) => (v === null ? '—' : `${v.toFixed(digits)}${unit}`);

  const stroke: Record<string, string> = {
    teal: 'rgb(var(--c-teal-400))',
    bedrock: 'rgb(var(--bedrock-400))',
    yellow: 'rgb(var(--c-yellow-400))',
    red: 'rgb(var(--c-red-400))',
  };

  function move(e: PointerEvent) {
    if (!svg || !points.length) return;
    const r = svg.getBoundingClientRect();
    const t = t0 + ((e.clientX - r.left) / r.width) * (t1 - t0);
    let best = 0;
    for (let i = 1; i < valid.length; i++) if (Math.abs(valid[i].t - t) < Math.abs(valid[best].t - t)) best = i;
    hover = valid.length ? best : null;
  }
  const hovered = $derived(hover !== null ? valid[hover] : null);
</script>

<div class="card !p-3">
  <div class="flex items-baseline justify-between gap-2 mb-2">
    <span class="text-[10px] uppercase tracking-wider text-deep-400">{label}</span>
    <span class="font-mono text-lg font-bold text-strong">{fmt(hovered ? hovered.v : latest)}</span>
  </div>
  {#if valid.length < 2}
    <div class="h-[140px] flex items-center justify-center text-xs text-deep-400 mc-slot">Waiting for samples…</div>
  {:else}
    <svg bind:this={svg} viewBox="0 0 {W} {H}" preserveAspectRatio="none" class="w-full h-[140px] mc-slot overflow-visible"
         role="img" aria-label="{label} over time" onpointermove={move} onpointerleave={() => (hover = null)}>
      {#each [0.25, 0.5, 0.75] as g (g)}
        <line x1="0" x2={W} y1={H * g} y2={H * g} stroke="currentColor" class="text-deep-600/30" stroke-dasharray="2 4" vector-effect="non-scaling-stroke" />
      {/each}
      {#each segments as seg, i (i)}
        <path d={area(seg)} fill={stroke[color]} fill-opacity="0.12" />
        <path d={line(seg)} fill="none" stroke={stroke[color]} stroke-width="1.5" vector-effect="non-scaling-stroke" stroke-linejoin="round" />
      {/each}
      {#if hovered}
        <line x1={x(hovered.t)} x2={x(hovered.t)} y1="0" y2={H} stroke="currentColor" class="text-deep-300" vector-effect="non-scaling-stroke" />
        <circle cx={x(hovered.t)} cy={y(hovered.v)} r="3" fill={stroke[color]} vector-effect="non-scaling-stroke" />
      {/if}
    </svg>
  {/if}
  <div class="mt-2 flex justify-between text-[10px] text-deep-400 font-mono">
    <span>{hovered ? new Date(hovered.t * 1000).toLocaleTimeString() : `avg ${fmt(avg)}`}</span>
    <span>peak {fmt(peak)}</span>
  </div>
</div>
