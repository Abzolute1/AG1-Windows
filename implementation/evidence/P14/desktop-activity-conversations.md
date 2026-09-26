# Readable activity and independent chats — 2026-09-09

This is a focused desktop update for local user review, not P14 release acceptance. Installation identity and verification are recorded in [the receipt](desktop-activity-conversations.json).

## Activity and source output

Assistant progress text stays visible instead of disappearing inside the work log. Each assistant stage has a collapsed activity summary with actual file-read, folder-listing, search and command counts. A pending call shows its action and target; failures remain visible in the summary. Expanding shows named calls and paths rather than detached calls followed by detached outputs. The Rust projection supplies recorded call ID/name/failure metadata; the renderer pairs by ID, never arrival order. Older unpaired records remain accessible as Tool output.

Raw output is displayed literally, never interpreted as Markdown or HTML. The viewer preserves indentation, starts with a bounded preview, and provides wrapping, Copy, Show full output and Show less. Only expanded tool results fetch their complete journal pages. Expanding activity releases automatic following and preserves the clicked row's reading position; Jump to latest restores following.

Screenshots: [collapsed progress](desktop-activity-conversations/activity-collapsed.png), [expanded calls](desktop-activity-conversations/activity-expanded.png), [literal HTML](desktop-activity-conversations/source-preview.png).

## Independent native conversations

Native desktop navigation selects among retained session drivers instead of replacing the executing session. Each conversation has its own journal selection, commands, progress and approval policy. Workspace authority, model admission, HTTP service and bounded worker pool remain shared. Stop targets the addressed conversation. Delayed view updates are fenced through the animation-frame callback so they cannot overwrite a later navigation.

Local requests share the existing endpoint queue; the sidebar distinguishes Queued, Working, Needs approval, Finished and Failed. Remote requests use the existing scheduler capacity. An idle conversation cannot change a project's repository while sibling conversations are active. Duplicate legacy resume into an already open owner is rejected. Up to 64 owners are resident; only idle, nonselected, non-root owners may be evicted, preserving durable history. Legacy TypeScript workspace switching still requires settlement. Concurrent edits to the same project are subject to existing tool revision checks; navigation does not imply isolated worktrees.

The private native WebKit test opens a second conversation while the first provider request is active, verifies local queuing, switches back, and checks both complete responses. Native tests independently run two requests, cancel only one, preserve separate approval modes, reject cross-scope and duplicate-resume actions, and reopen history. No actual API account or GPU inference was used.

## Scrolling

Unchanged transcript turns remain memoized. Live observation changes no longer force the transcript's synchronous layout effect; content growth follows ResizeObserver and requestAnimationFrame. Tool hydration and output DOM mounting are demand-driven.

Local scroll diagnostics retain at most 20 bounded animation-frame samples in the desktop state's rendering-samples.jsonl. They contain timing/count/viewport data only. They measure callback cadence, not compositor presentation. Private Xvfb samples verify collection but do not establish physical 165 Hz smoothness. Windows/macOS measurements remain outstanding; platform-neutral frontend changes do not constitute platform performance acceptance. The existing Linux X11/DMA-BUF compatibility workaround is retained.

## Verification and rollback

55 frontend tests, 242 native session tests, the independent-conversation contract, 34 storage contracts, 54 desktop bridge/cutover contracts, four desktop unit tests and 18 protocol contracts pass. Registry/task-guidance contracts and generated protocol drift checks also pass. Native private visual tests report no JavaScript errors and confirm readable literal output, lazy expansion, preserved progress, queueing and independent responses. Production launcher verification is recorded separately in the receipt.

The preceding installation is /opt/ag1/releases/16e04b2ec9e945bcb3706672df73f4b1. The source rollback guard covers 173 files, including regenerated protocol fixtures; it refuses later edits. Per-conversation selection metadata does not require a database schema migration; journals remain retained. Older native builds ignore the separate conversation-selection namespace and retain their previous navigation behavior. See [rollback instructions](../../DESKTOP-UI-ROLLBACK.md). No duration workload was started and no new high-refresh or model-latency benchmark is claimed.
