# Chat activity placement and website icons — 13 September 2026

## Outcome

Implemented and packaged. Systemwide installation is pending because the noninteractive installer returned `sudo: a password is required`. The currently installed release remains `/opt/ag1/releases/ef76f6b16b06446e8df7167f3b696bdf`. Run [install-verified.sh](install-verified.sh) locally to authenticate and install the verified production archive, then reopen AG1.

The active status and live thinking block now follow the conversation's completed tool work, prominent results, change cards and messages. Preserved reasoning becomes “Thoughts” when answer text or a tool call starts. Active work uses the conversation scroll area instead of a separate capped inner scrollbar, and collapsing work does not hide current status.

Successful native web search/fetch results now show website badges and source counts. Opening the row exposes titled source links, with exact arguments and output retained. Icons load lazily through the native backend, are bound to URLs in the selected chat's tool records, and are cached and bounded. Public HTTPS fetching revalidates DNS/redirect targets; private-network targets, credentials and nonstandard ports are rejected. Raster decoding returns small PNG data URLs; the renderer CSP remains unchanged. Missing icons use a hostname initial. Source counts use neutral text, and loaded icons have a light backing for readability.

## Verification

- **271 frontend tests passed**, including status order after tools and prominent result cards, collapsed work, reasoning transitions, source badges, request sharing and fallback behavior: [final log](final-ui-after-review.log).
- **290 native checks passed**: 268 session contracts, 1 conversation contract, 2 thinking contracts, 16 web HTTP contracts and 3 icon/source/decode units: [contracts](final-native-contracts.log), [icon units](final-native.log). The first icon-unit command also selected integrations with a name filter and ran zero integration tests; the separate contracts command ran all 287 integrations. Distinct final frontend/native total: **561**.
- Production TypeScript/Vite build and native/package audits passed. All **1144 packaged files**, their sizes/modes, and archive hash verified in the isolated installation: [package](package.json), [installer check](package-validation.json), [final verification](final-verification.json).
- Two bounded **Qwen3.6-35B-A3B-UD-Q4_K_XL** runs used the existing tuned runtime configuration: Rust documentation, 6 tools / 73.847 seconds; GitHub, 2 tools / 41.09 seconds. Both completed, and all observed live indicators followed completed tools. Real Rust and GitHub favicons loaded. [Rust run](native-rust/result.json), [GitHub run](native-github/result.json).
- Native Linux Tauri/WebKit screenshots were taken only on authenticated private Xvfb displays. Inspected initial/ongoing thinking, actual website badges, expanded sources, and 1440×900 / 900×600 layouts. Neither layout overflowed the page horizontally. [Thinking below work](native-github/thinking-after-search.png), [wide source details](native-github/sources-1440.png), [narrow source details](native-github/sources-900.png).
- The final renderer-only adjustment moved status/live output after prominent cards. Its dedicated regression and the complete 271-test frontend suite passed after the real-model captures; those captures predate that last ordering adjustment. The exact final production bundle was subsequently rebuilt and verified by the package audit/isolated installer. No claim of a third real-model run on the final archive is made.
- Protected model profiles, runtime capture and original process-host bytes are unchanged. Firefox PID 24879 is unchanged. All owned desktop, display and llama processes are stopped. No desktop/YouTube capture, ChatGPT comparison, duration run or user-project mutation was performed.

## Limits and retained attempts

Both real web-search exercises returned no results (including the broad “GitHub” query). Direct fetching worked. The exact cause of the empty search responses was not diagnosed in this presentation task. Successful search-result metadata/icon rendering is covered by native projection units and frontend fixtures; live icon/source coverage came from real webFetch results. Empty searches retain the generic search icon, without invented sources. Broader chat/artifact closeout and release acceptance remain separate work.

The first native attempt failed before any prompt because the fixture had not opened an isolated project; [log](native-live.log). That setup was corrected. The first package build was stopped to correct propagation of the six-job build limit; [retained log](package.log). The first real-model completed-work screenshot raced the final collapse, so the second driver explicitly waited for renderer settlement. Visual review then corrected red source counts and dark-logo contrast. Earlier packages and logs are retained as intermediate evidence, not the install target.

## Package and recovery

- Production archive: `.local/p14-packages/912252da163d400f85c858ad7e225450/ag1-0.1.0-rc-linux-x86_64.tar.gz`
- SHA-256: `2d85abedfc7a3f33e036dd7202c3669ee8210a6ca35f333e45497220bd4c89ce`
- Isolated verified release: `/home/alex/Projects/AG1/.local/chat-activity-install-check-20260913/releases/2f1cac2ac3ce41c0af0e54b98c7f2d90`
- [Exact task source patch](changes.patch), [before hashes](source-before.json), [original source archive](source-before.tar.gz), [after hashes](source-after.json).
- [Installation attempt](install.log) did not modify `/opt/ag1/current`; [ready installation command](install-verified.sh) pins the verified archive and hash. Visual-test binaries are not installation targets.
