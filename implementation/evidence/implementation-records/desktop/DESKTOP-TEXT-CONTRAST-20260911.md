# Desktop text contrast — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user requested brighter, more readable text after comparing AG1 Midnight with Codex. This pass changes only `product/apps/desktop/src/builtin-themes.json` and `product/apps/desktop/src/redesign.css`.

Dark built-in themes use brighter primary and secondary text. Midnight primary text changes from `#c9d1d9` to `#f0f3f6`, and secondary text from `#8b949e` to `#b1bac4`. Graphite and Tokyo receive matching readability improvements while retaining their palette character. Accent/secondary foregrounds remain consistent with the primary text. CSS fallback palettes match the built-in theme data. Search and composer placeholders use the secondary color at full opacity.

Backgrounds, accent colors, layout, font sizes, disabled-control opacity, the light theme and custom theme palettes are unchanged. There are no changes to model settings, tools or CV.

Verification: frontend TypeScript/Vite build and two existing theme tests passed. All 30 dark-theme primary/secondary comparisons against the five main surfaces improve and exceed 4.5:1. Midnight text against the main background increases from 12.26:1 to 16.99:1 (primary), and 6.15:1 to 9.63:1 (secondary). These are palette contrast calculations; rendered antialiasing varies with display settings.

Evidence and exact pre-change files: `.local/text-contrast-20260911/`. Production build uses the separate `text-contrast-production-20260911` profile, preserving previous accepted binaries. Native visual checks use an authenticated private Xvfb display and disposable app state; no user desktop capture or real model requests.

Status: **installed and verified**. Current release: `/opt/ag1/releases/f1f0ec5c982c4a04b2853c1213f4847e`. All 1,142 installed payload files, receipt and protected runtime/CV hashes match. The packaged app was visually reviewed before installation on the private display; brighter text is visible with unchanged layout and background. Exact package SHA256: `b83226cd65e35626fc68367f920548934eef3d928c780402dbefda664a90f862`.

[Machine evidence](../../P14/desktop-text-contrast-20260911.json), [before](../../../../.local/text-contrast-20260911/before/home.png), [after](../../../../.local/text-contrast-20260911/after/home.png). No user window was closed or captured. Fully quit and reopen AG1 to load the change. Prior release `fab3e5b44e114c51827453cdb3732951` remains available.
