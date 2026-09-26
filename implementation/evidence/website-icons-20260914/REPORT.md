# Website icon reliability — 14 September 2026

Built, validated and installed systemwide at `/opt/ag1/releases/0204babfa8784376801d9a746ab7a2f3`. Reopen AG1 to use the update. The running user app was left open. The original llama.cpp installation, model configuration and tuning were not changed.

Website logos cannot be guaranteed when their publishers and icon services are unavailable. AG1 now always has a themed local globe fallback while improving recovery of the real logo. A fallback is never represented as the website's actual branding.

## Delivered

- Discover HTML-declared icons, including relative paths, document base URLs and separate CDN hosts, in addition to conventional favicon/apple-touch paths. Strip comments/script/style content during discovery and bound candidate count and HTML size.
- Accept compressed HTTP responses with decoded-body limits. Every destination and redirect remains HTTPS-only, public-address validated and DNS-pinned, with bounded time, bytes and redirects. No browser cookies, credentials, page URL or search text are sent to an icon service.
- Support static SVG favicons through the existing constrained native SVG renderer. Only a metadata-free PNG reaches the WebView. Active content and external resources remain refused; icon rasterization is capped at 64px and shares decoder admission.
- If direct discovery fails, try DuckDuckGo's icon service using only the validated public hostname. This is optional decoration and does not change search results or model evidence.
- Queue up to four concurrent frontend requests instead of dropping bursts. Repeated hosts share pending requests. Native saturation is a retryable error and does not create a negative cache entry.
- Expire failed lookups after 30 seconds. Visible icons retry up to twice, reconnecting triggers recovery, and unmounting stops retry timers. Cached successful native logos survive a failed refresh. Caches are bounded in memory, not a permanent on-disk/offline asset library.
- Replace letter badges with the app's globe icon whenever no usable image is available. Existing circular stacks and light/dark styling remain intact.

## Evidence and checks

The preserved original fetch implementation was run alongside the new implementation against six public sites. **OpenAI reproduced the reported failure: old `NotFound`, new logo successfully decoded.** Both implementations succeeded on the other five sites. New direct retrieval succeeded for OpenAI, Python docs, GitHub, Hacker News, DuckDuckGo and Findskill. The separate domain-only fallback was also explicitly exercised successfully for OpenAI. These are live checkpoint results, not a future availability guarantee. [Comparison log](live-comparison.log).

- **279 frontend tests passed**, including queue saturation, coalescing, automatic retry, reconnection, unmount cleanup and unsafe-response fallback. [Full frontend log](frontend-all.log).
- **8 focused native icon checks and 5 image/SVG checks passed**, covering discovery, URL boundaries, active-content rejection, negative-cache expiry, stale successful logos and saturation. [Icon checks](native-final.log), [image checks](image-tests.log). The live smoke is an additional explicitly invoked ignored test.
- TypeScript/Vite and production Rust builds, bundle audit and isolated package installation passed. [Frontend build](frontend-build-final.log), [package validation](package-validation.json).
- Actual `SiteIcons`/`SourceList` components rendered the native-fetched PNGs in private Chromium in both themes. All nine displayed image instances decoded at 16 × 16, the local fallback remained visible and there was no horizontal overflow. [Measurements](visual.json), [dark](dark-icons.png), [light](light-icons.png). These are isolated component checks with live-result replay, not a fresh native WebKit/model run. No user desktop was captured.
- **All 1,144 installed payload files** matched manifest hashes, sizes and modes. The archive receipt matched, CLI version smoke passed, all **114 protected runtime/configuration hashes** stayed unchanged, and packaged-source inputs still matched the retained final source hashes. [Installation verification](systemwide-verification.json).

An initial new test needed an explicit TypeScript promise-result type. The visual probe initially used an incorrect CommonJS import and omitted the workflow stylesheet; those probe issues were corrected before the final theme inspection. Initial logs are retained alongside the successful checks.

## Reproduction and rollback

[changes.patch](changes.patch), [complete original source archive](source-before-complete.tar.gz) and [final source hashes](source-after.json) identify the five implementation/test files changed: `WebSources.tsx`, `WebSources.test.tsx`, native `client/website_icons.rs`, `image_tools.rs` and `image_tools/svg.rs`. No dependency or lockfile changes were needed.

Use the repository's Node 24 toolchain for frontend tests/build. Native checks use `cargo test --offline --locked --manifest-path product/Cargo.toml -p ag1-platform --lib website_icons -- --test-threads=1` and the same command filtered to `image_tools`. The public-network comparison is opt-in via `live_website_icon_smoke -- --ignored --nocapture --test-threads=1`; `AG1_ICON_SMOKE_DIR` optionally saves its sanitized PNGs. The legacy fetcher is retained only under `cfg(test)` for reproduction.

Scratch scripts and logs are under `.local/website-icons-20260914/`; copies of the package and visual scripts are retained here. Reconcile historical paths and use a fresh evidence directory before repeating a build. The package archive is `.local/p14-packages/8b1b29bb888444c2a1065d222dd006e5/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `bc65c9704ea7bf52fc3446dd85aef33ae1502ca96a3fd6c975b58055a1e2bcb4`.

The installer retained the previous release, `ee7da595f62146489bc06da06be93e10`, in its receipt for reversible application rollback. No llama.cpp runtime cutover, model inference, search-engine behavior change or broader feature/endurance acceptance is part of this delivery.
