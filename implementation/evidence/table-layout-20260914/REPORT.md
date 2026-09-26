# Markdown table wrapping repair — 14 September 2026

Fixed and installed systemwide at `/opt/ag1/releases/34e2feafc5ff4a4d9464a06a80af5777`. Reopen AG1 to load the new renderer; existing saved answers use the corrected styling without regenerating them.

## Cause and change

The saved assistant answer in session `chat-b9116a09030da1b8e4993fc3bbf6b056` contains valid GFM tables. Chat-level `overflow-wrap:anywhere` was inherited by table cells, allowing long description columns to compress short labels to individual glyphs. The short final summary table did not trigger the same pressure.

Only `product/apps/desktop/src/markdown.css` changed in application source: tables override wrapping to normal word boundaries, retain the existing horizontal overflow area, and cells align to the top. No column count, weapon-specific sizing or model-output workaround was introduced. Words can wrap at natural spaces/hyphens; long unbroken tokens scroll inside the table.

## Verification

- Read the saved response through read-only SQLite access; retained it in `response.md`.
- Reproduced broken Rank text before the fix, then checked the saved answer at 1150, 768 and 390 px in headless Chromium. Whole-word Rank/KTS100 measurements passed afterward; a 240-character token scrolled locally without page overflow.
- Repeated all three widths using the final compiled frontend CSS alone. `bundled-layout.json`, `built-css.json` and `bundled-after.png` record the result. Visually inspected the private rendered screenshot; no user desktop capture.
- Existing Markdown tests: 3 passed. Frontend TypeScript/Vite build passed; Vite reported bundle-size advisories.
- Normal production native build, bundle audit and isolated installer validation passed. All 1144 installed payload files matched manifest hashes, sizes and modes; receipt matched the final archive. Installed CLI version smoke passed.
- Protected desktop configuration, model profiles, repository supervisor and the previously restored pinned supervisor hashes are unchanged. No model run, backend/weight edits or live chat mutations.

The layout measurements use Chromium, not a fresh native WebKit desktop session. The running user app was left open and will retain its previous renderer until reopened. Broader chat cleanup and release/endurance work remain outside this repair.

## Packaging correction

The initial package reused a stale frontend because the Cargo packaging helper does not invoke Vite. It was briefly installed, then replaced after the compiled-CSS check caught the omission. `superseded-stale-frontend/` retains that attempt's records. A fresh frontend build preceded the final native package; the final compiled CSS and installed payload verification passed. No old release dependencies were deleted.

Final archive: `.local/p14-packages/e364cab9c23148599dedb3c3ebd75156/ag1-0.1.0-rc-linux-x86_64.tar.gz`; SHA-256 `37c327d131ddbf43c577d74d33a04f8f08bf6dc96140c781f1095bcc524ef811`. Exact task changes are in `changes.patch`; pre-edit source/navigation bytes are in `before.tar.gz`.
