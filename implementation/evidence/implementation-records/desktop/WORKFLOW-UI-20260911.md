# Compact response workflow — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Status: **installed and verified**, release `/opt/ag1/releases/669b538d78684675b09a17643924d7fb`. Production build, package validation and all 1,144 installed payload files/receipt/protected hashes passed. Previous release `dd94f46abadc4e9cae7e7a22d5f5d99a` is retained. Weekly usage snapshot: 4% remaining. Restart AG1 to load the new renderer, including for existing messages.

The chat now groups all committed work for a response under one expandable header. It opens automatically during execution and collapses after completion unless the user explicitly chose its state. Progress commentary and compact tool rows remain chronological inside that section; the final answer stays outside. Rows reveal arguments and literal output on demand, retaining deferred journal hydration and exact call/result pairing.

The current response uses the existing measured UI timer for “Working for …” / “Worked for …”. Old responses without available timing use “Work details”; this change does not fabricate or backfill historical durations. The existing appearance setting controls whether elapsed time is shown. The duplicate composer timer is hidden for native conversations. The noncompact preference opens work by default, while the user can still collapse it.

GitHub discovery and read results no longer create separate cards between every tool step. They appear as short named rows such as “Listed repositories” and “Read repository”, with details and existing link/review actions inside the row. Confirmed changes, uncertain/failed GitHub outcomes, recovery/download outputs and successful image results remain separately accessible. General tool failures are counted on the header even while collapsed. Existing approval UI, change-review and output-review controls remain in place.

The scoped CSS uses quiet icons, muted labels, restrained spacing, a single divider and a bounded detail area. It does not change execution, permissions, CV or provider behavior.

Validation: 50 tests passed across conversation, tool activity, GitHub results, retained outputs and app integration. Coverage includes multiple assistant rounds, automatic completion collapse, exact reverse-order call/result pairing, lazy large-output hydration, live answer/thinking behavior, uncertain GitHub receipts, approvals and app interactions. Six model-picker App tests initially failed both with the change and with the original App component. Their cause was cross-test browser-storage state; clearing local/session storage before each App test resolved all six. Original failing, baseline and passing logs are retained.

The frontend TypeScript/Vite build passed. An owned headless browser checked running, collapsed completion and expanded result views in dark/light themes at 1,100 pixels and midnight at 430 pixels: one work header, four compact rows, working expansion, no page overflow or browser errors. Screenshots were visually reviewed. No user desktop was captured or interrupted, and no real model/GitHub operations were needed for this presentation change.

Evidence directory: `.local/workflow-ui-20260911/`, including original files, scoped patch, tests, visual fixtures, screenshots and package/install verification. [Machine summary](../../P14/workflow-ui-20260911.json).
