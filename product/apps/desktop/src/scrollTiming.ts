import { invoke } from "@tauri-apps/api/core";
/** Bounded, local-only frame timings. No transcript, paths or input is recorded. */
export function observeScrollTiming(element: HTMLElement | Document): () => void {
  let frame = 0, lastScroll = 0, previous = 0, cooldown = 0;
  const samples: number[] = [];
  const finish = () => {
    frame = 0; previous = 0;
    if (samples.length >= 12) {
      const sorted = samples.slice().sort((a, b) => a - b);
      void invoke("record_rendering_sample", { sample: { frames: sorted.length, mean_ms: samples.reduce((sum, value) => sum + value, 0) / samples.length, median_ms: sorted[Math.floor(sorted.length * .5)], p95_ms: sorted[Math.floor(sorted.length * .95)], max_ms: sorted.at(-1), over_25_ms: sorted.filter(n => n > 25).length, width: innerWidth, height: innerHeight } }).catch(() => {});
    }
    samples.length = 0; cooldown = performance.now() + 3000;
  };
  const tick = (now: number) => {
    if (document.hidden) { samples.length = 0; finish(); return; }
    if (previous) samples.push(now - previous);
    previous = now;
    if (now - lastScroll > 180 || samples.length >= 180) finish(); else frame = requestAnimationFrame(tick);
  };
  const scroll = () => { lastScroll = performance.now(); if (!frame && lastScroll >= cooldown) frame = requestAnimationFrame(tick); };
  element.addEventListener("scroll", scroll, { passive: true, capture: true });
  return () => { element.removeEventListener("scroll", scroll, true); cancelAnimationFrame(frame); samples.length = 0; };
}
