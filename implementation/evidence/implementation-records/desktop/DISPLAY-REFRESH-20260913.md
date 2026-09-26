# Display refresh-rate repair — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Complete and installed systemwide. Restart AG1; both launchers use the new build. The user's running instance was left open. Measurements used separate owned windows on the real monitor, with no chat or model requests.

## Root cause and earlier incomplete fix

The physical BenQ display is configured at 165.002 Hz under Hyprland/Wayland with an NVIDIA RTX 4080. The previous launcher forced X11 and disabled WebKit's DMA-BUF renderer. Its code disabled `PreferPageRenderingUpdatesNear60FPS`, but that setting could not repair the actual clock source. A direct read-only DRM probe found the connected 2560×1440 CRTC and reproduced `drmWaitVBlank` returning ENOTSUP. WebKit falls back to its approximately 60 Hz timer when it cannot create a DRM display clock. See [WebKit monitor selection](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/glib/DisplayVBlankMonitor.cpp), [DRM implementation](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/glib/DisplayVBlankMonitorDRM.cpp), and [timer implementation](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/glib/DisplayVBlankMonitorTimer.cpp).

Actual on-monitor baseline measurements with WebKitGTK 2.52.5 and the existing disabled near-60 preference:

| Configuration | Animation callback rate | Median / p95 |
| --- | --- | --- |
| X11, previous compatibility flags | 62.16 FPS | 16 / 17 ms |
| Wayland, previous compatibility flag | 62.09 FPS | 16 / 17 ms |
| Wayland, shared-buffer compositing | 62.16 FPS | 16 / 17 ms |
| Wayland, shared buffers + new clock adapter | 165.00 FPS | 6 / 7 ms |

The unmodified DMA-BUF path caused a Wayland protocol error on this machine. Shared buffers rendered successfully. GTK's own Wayland frame callbacks already followed the high-refresh monitor, which provided the usable fallback clock.

## Delivered change

AG1 prefers native Wayland when available and uses shared-buffer WebKit compositing on detected NVIDIA hardware, while respecting explicit environment overrides. The system launcher no longer forces X11 or the older renderer-disable workaround.

A small C adapter is statically linked into the AG1 executable, exporting the public `drmWaitVBlank` ABI. It always calls real libdrm first. Only unsupported synchronous relative waits of zero or one refresh, without signal/event requests, can use AG1's Wayland GTK frame clock. Successful driver calls and unrelated failures pass through. It captures the input union before calling libdrm because the failing ioctl can overwrite it. No LD_PRELOAD, system-library replacement, driver change, device permission change, or fixed FPS override is used.

The fallback wakes on GTK frame callbacks and stops its tick subscription after rendering becomes idle. It tolerates hidden-window pauses and retires with the widget. `AG1_DISABLE_DISPLAY_CLOCK_FALLBACK=1` disables it; explicit `GDK_BACKEND` overrides remain available. The controlled production-adapter probe confirmed zero additional adapter ticks during the idle observation interval.

The existing bounded local scroll diagnostics now observe all app scrolling, including Settings and MCP, and record mean callback intervals as well as median/p95. They collect no transcript or input contents. Native diagnostics validate the timing fields and retain at most twenty records as before.

## Production verification and limits

The normal packaged AG1 was launched with owned state/provider/authority on the physical Wayland monitor. Its Appearance Settings scroll produced 180 callback samples: median 6 ms, p95 7 ms, maximum 7 ms, zero gaps over 25 ms. The test intended to navigate to MCP but clicked Settings because native Wayland window decorations shifted the target; screenshot inspection identifies the measured view accurately. No user settings were changed. No model requests occurred; no owned child processes leaked.

The controlled animation result is 165.002 FPS. Callback timing is not an external measurement of every presented pixel, and busy views can still drop frames. The raw app sample's quantized callback timestamps yielded a 5.633 ms mean; that is retained, not represented as a claim that the 165 Hz monitor presents 177 FPS. Multi-monitor moves, other GPU drivers and suspend/resume were not independently exercised. The existing X11 private-display production smoke still passes, but the fallback is specifically for Wayland.

Typecheck, renderer build, offline/locked production build and normal package validation passed. Evidence is under `.local/refresh-rate-fix`: backend comparisons, `probe.c`, the read-only DRM probe, `wayland-production-adapter.log`, `physical-app/result.json`, screenshots, and installation receipts. Disposable build intermediates from an earlier UI build were removed to recover space; previous binaries, archives and installed releases remain retained.

Installed release: `/opt/ag1/releases/06ef4dab7bdd44ed8572862327745531`. Desktop SHA-256: `141b3b514969ce2980d1859fe68ee1b34ca507a126da4af77255bd1dee89fef6`. Archive SHA-256: `9351a6d214b24832a9fe300a667ff31b45e82015ab1315b5c8df7b824e1b8e03`.
