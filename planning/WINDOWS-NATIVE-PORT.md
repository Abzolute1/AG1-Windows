# AG1W Windows-native port

## Goal

Build the AG1 desktop runtime for Windows without WSL, a Linux process host, or
the Linux-only TypeScript compatibility worker. The Linux implementation stays
unchanged. Windows code must use a separate backend wherever the OS semantics
are different.

## What the initial map found

The core crate, protocol, storage, provider HTTP client, llama launch-plan
parsing, and desktop web assets are already portable. The platform crate is
not. Its Linux implementation relies on `openat2`, file-descriptor paths,
`flock`, inotify, `/proc`, pidfds, Unix sockets, PTYs, GTK private dialogs,
and Bubblewrap. Removing the Linux guards would either fail to compile or,
worse, weaken containment without making it visible.

## Port order

1. **Windows foundations** — add Windows-specific workspace and process-tree
   implementations. Workspace code will reject reparse-point traversal and
   retain revision/CAS checks. Process code will own children in a Windows Job
   Object so closing AG1W terminates its local server and descendants.
2. **Platform composition** — compile the shared session, file, source, Git,
   resource, MCP, local-model, and client modules against those foundations.
   A feature cannot be enabled until it has a Windows implementation; it must
   return a clear unavailable result rather than pretending to run.
3. **Desktop shell** — replace GTK-only folder and private-input paths with
   Tauri/Windows equivalents, move state protection to Windows ACL-aware
   storage, and remove the Linux compatibility-worker startup path.
4. **Native llama runtime** — launch the Windows llama.cpp binary as an owned
   child, preserve the captured model/launch-plan checks, and stop it when
   AG1W closes. This phase does not load a model during build validation.
5. **Parity verification** — split Linux-specific contracts from portable
   contracts, add Windows fixtures for filesystem/process behavior, build the
   desktop bundle, and run a no-inference smoke test.

## Boundaries

- Do not edit `/home/alex/Projects/AG1-Windows`, AG1-Arch, Turtle, or any
  model/runtime installation.
- Do not silently fall back to WSL or launch a model.
- Linux sandbox semantics cannot be claimed on Windows. The Windows backend
  will use Windows containment where available and will refuse operations that
  do not yet have an equivalent, rather than running them less safely.
- This document records the implementation order, not a claim that all phases
  have finished.

## Current implementation record

**W1 foundation implemented — not yet exposed to the desktop.** `workspace_windows.rs` has a Windows file
identity implementation based on `GetFileInformationByHandle`, directory
handles opened with `CreateFileW(FILE_FLAG_BACKUP_SEMANTICS)`, session-local
named-mutex admission, bounded reads/streams, revision-checked replacement, and a
bounded non-reparse-point scan. The workspace/source slice compiles on the
Windows target. Its focused tests cover read, CAS replacement, stale-write
rejection, and scan filtering.

`process_windows.rs` now creates each owned child suspended with `CreateProcessW`,
passes only its three standard-pipe handles through an explicit handle list,
assigns it to a `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object, then resumes it.
It uses a distinct job per launch, so stopping or dropping one owned child ends
that child and its descendants without touching another launch. The focused
tests verify bounded log capture and job-tree termination using short-lived
`cmd.exe` fixtures. They do not start llama.cpp or load a model.

**W2 local-model runtime implemented — not yet exposed to the desktop.**
`llama/native_windows.rs` verifies immutable runtime materials, launches the
exact selected executable as an owned Job Object child, and uses the held
process handle for liveness instead of trusting a PID. The verifier rejects a
same-size material replacement. Its lifecycle test uses a sealed `cmd.exe`
fixture, verifies that the owned child becomes live, then stops it. It does not
start llama.cpp, bind a port, or load model weights. GPU-exclusivity checks and
`/proc` resource reporting deliberately return unavailable on Windows until a
real Windows equivalent is implemented. The private runtime directory rejects
reparse points; ACL parity is still part of the desktop-state work.

The Windows backend is not exposed to the desktop yet. Bidirectional protocol
pipes, PTY terminals, Linux descriptor-pinning, Bubblewrap containment, MCP,
session/client, GitHub integration, and the Linux compatibility worker remain
unavailable on Windows. This is intentional: exposing a chat path that looks
native but cannot reliably own its processes or reproduce a containment
guarantee would be misleading. The next slice is Windows-specific desktop state
and native-client composition, then a real model smoke test after that path is
coherent.

**W3 workspace services implemented — not yet a session host.** The native
authority registry now keeps per-workspace records under the Windows local
app-data tree, holds both a named OS mutex and an in-process guard, verifies
workspace identity with volume/file-index values, and uses write-through
replacement for records. A legacy rollback marker is an explicit refusal:
AG1W will not wake the old TypeScript worker or claim it has recovered that
state. Its registry fixtures cover clean shutdown/reopen and loopback endpoint
ownership.

The Windows workspace now supports stable conversation identities, bounded
directory listing, checked file creation, and checked replacement. Checked
writes keep the cancellation fence before publication, reject reparse-point
paths, use a Job-independent workspace claim, and verify their published bytes.
Folder grants compile and pass their own Windows fixtures: their persistent
records use a synced temporary file plus `MOVEFILE_WRITE_THROUGH`, and normal
Windows paths are normalized against the `\\?\` spelling produced by
`canonicalize`. The Linux-only `/mnt` discovery feature remains Linux-only.
The native file-tool module and change receipts compile against these Windows
operations, but they are not exposed until the Windows session adapter is
implemented.

Validation so far is `cargo test --offline --locked -p ag1-platform --lib`:
37 Windows tests pass. These tests use temporary files and `cmd.exe` fixtures
only. They never launch llama.cpp, bind a model endpoint, load weights, or
start the desktop UI.

**W4 durable session and storage base implemented — not yet a provider host.**
`ag1-storage` no longer rejects Windows at open. A Windows store now holds an
OS mutex derived from the canonical private-store path, treats reparse points as
unsafe in every checked ancestor, and publishes immutable blobs through a
synced staging file plus `MoveFileExW(MOVEFILE_WRITE_THROUGH)`. SQLite continues
to run in WAL/FULL synchronous mode. Directory flushes use Windows directory
handles; the two Windows filesystem responses which document directory-flush
unsupported are accepted only after the synced/write-through publication.

`session_windows.rs` is a separate, deliberately small owner around the real
`ag1_core::runtime::SessionRuntime`: it generates cursor keys with Windows CNG,
bootstraps/resumes AG1 v3 journals, acknowledges only the exact SQLite commit
receipt, and compiles the existing context-virtualization projection through
`ContextStore`. Its tests create a durable chat, append a user message, retain
the original journal/request context artifacts, resume the session, and prove
that a selected context limit survives a restart. It contains no replacement
chat loop and does not start a provider or model.

Lossless archive export/restore is now available on Windows.
They use a sibling staging directory plus `MoveFileExW(MOVEFILE_WRITE_THROUGH)`
without `COPY_ALLOWED` or `REPLACE_EXISTING`. That keeps publication on one
volume, refuses a pre-existing destination, and prevents a copy/delete fallback
from weakening the atomic boundary. The first implementation failed with
`AccessDenied`; the cause was AG1W's still-open `export.json` handle. The export
path now closes that handle after syncing it and before publishing the directory.

Current validation is `cargo test --offline --locked -p ag1-storage --lib`
(6 pass, 1 process-crash child intentionally ignored),
`cargo test --offline --locked -p ag1-storage --test storage_contract`
(42 pass), and `cargo test --offline --locked -p ag1-platform --lib`
(40 pass). They use temporary files, a loopback HTTP fixture, and short
`cmd.exe` fixtures only. No local model, llama.cpp server, or desktop UI was
started.

**W5 native local-provider session implemented — not yet a desktop host.** The
Windows session owner can now perform one actual local llama.cpp turn through
the portable provider transport. It retains AG1's required order: append the
user message, snapshot/compile the existing virtualized context, acquire the
single local scheduler permit, admit the request, stream the provider response,
retain raw provider output as custody evidence, then append the assistant only
after transport settlement. A scheduler rejection or cancellation explicitly
discards the preparation rather than leaving the session blocked.

This path accepts only `llamacpp` OpenAI-compatible models on a loopback URL
and rejects any configured tools. That is intentional: the Windows tool runner
is not composed yet, so advertising tools would make a model believe it can
perform work which the native host cannot execute. The desktop will be wired to
this local-only session path before tools are added.

The provider test uses a one-request loopback SSE fixture. It proves a full
durable exchange—including context artifacts, scheduler admission, streaming
decode, raw-output artifact custody, and a resumed two-message journal—without
starting llama.cpp or loading a model. The legacy storage output-origin path now
also generates its cursor key with Windows CNG rather than `/dev/urandom`.

Current validation is `cargo test --offline --locked -p ag1-storage --lib`
(6 pass, 1 process-crash child intentionally ignored),
`cargo test --offline --locked -p ag1-storage --test storage_contract`
(42 pass), and `cargo test --offline --locked -p ag1-platform --lib`
(40 pass). No real local model, llama.cpp server, or desktop UI was started.

**W6 Windows desktop composition implemented — still not parity-complete.**
Windows now uses its own desktop entry point and native local host rather than
the Linux compatibility worker. It opens AG1's durable session, compiles the
real context-virtualization projection, retains raw provider output, and can
either attach to a configured loopback llama.cpp server or, when a sealed
runtime capture is configured, own the exact Windows llama.cpp child inside a
Job Object. The host exposes explicit model loading, cancellation, and shutdown;
the title-bar close path waits for the owned runtime shutdown.

The desktop catalog now persists with synced, write-through publication. A
sidebar chat reopens its actual durable journal after restart. New journals
retain the selected local model and system prompt in their header, so changing
the picker cannot silently alter an earlier conversation when it is resumed.
The config and catalog files are size-bounded and published through synced
temporary files plus `MoveFileExW(MOVEFILE_WRITE_THROUGH)`.

Validation: the Windows platform library suite is **41/41 passing** when run
outside Codex's AppContainer sandbox; two localhost fixture tests are blocked
only by that sandbox and pass unchanged outside it. The direct desktop host
fixture now proves a streamed local turn, restart, journal resume, and retained
original model selection. The sidebar persistence fixture passes. These tests
start only temporary loopback fixtures or `cmd.exe`; they do not start
llama.cpp, load a model, or consume GPU memory.

Remaining work before any parity claim: an automated real-model launch and
shutdown smoke test; native adapters for process/terminal tools, MCP, GitHub,
and the remaining file-workspace actions; complete session-history branching;
and a Windows equivalent to each Linux containment guarantee. The sealed
runtime setup, Windows folder picker, durable chat catalog, and narrow
project-file tool set are already exposed. Linux remains unchanged.

**W7 native vision and recovery completed — still not parity-complete.** A
configured local model now discovers a single sibling `mmproj*.gguf` projector
next to its selected GGUF without loading, restarting, or probing the model.
An explicit projector still wins; zero projectors keep the model text-only;
multiple candidates are rejected until the user chooses one. The resolved
projector is sealed with the model/runtime material and makes the published
model capability exactly `text,image`.

Pasted images are received only through the bounded, in-memory attachment
protocol. The host validates type, bytes, dimensions, and count before
retaining the message in the durable journal. The conversation UI keeps a
reference in the live view and fetches original image bytes only when it needs
to render a preview. The provider fixture proves the resulting local
OpenAI-compatible llama.cpp request uses the correct `image_url` data URL
shape. This test uses a temporary loopback fixture; it does not start
llama.cpp or load any model weights.

The Windows-native recovery command now reads the existing Windows authority
registry instead of always returning an empty result. It is read-only and makes
an interrupted handoff visible to the desktop recovery UI.

Current validation on this Windows checkout: `cargo test --offline --locked
--target-dir target-ag1w -p ag1-platform --lib` (49 passed), `cargo test
--offline --locked --target-dir target-ag1w -p ag1-desktop --lib` (21 passed,
including durable local turn, cancellation, live state, vision projector
discovery, pasted image request, and approval change), focused React tests
(36 passed), `npm run typecheck`, `cargo check` for the desktop binary, and
`git diff --check`. The loopback fixtures are test-only; no model process was
started for this validation.

**W8 narrow project-command tool completed — not a terminal replacement.** A
project chat in an explicit allow-changes mode now has one additional native
tool: `run_project_command`. It runs only a direct `cargo`, `dotnet`, `git`,
`node`, or `python` child in the opened project with a separate argument list.
There is no shell command string, no `cmd` or PowerShell, no piping/redirection
or chaining, and no Node/Python eval flag. The child receives a small explicit
environment allowlist rather than the desktop's whole environment.

Each invocation is owned by a Windows Job Object, has a maximum 120-second
run, captures at most the last 64 KiB of combined output, removes its private
spool file after completion, and ends the full child tree if it times out. It
is intentionally absent from read-only and general chats. The purpose is to
support ordinary build/test/status work now without pretending AG1W has the
interactive PTY terminal or broad process capability of the Linux build.

Validation now includes the direct-child fixture: it runs `cargo --version`,
proves output and exit-code capture, confirms cleanup, and rejects both `cmd`
and Python `-c`. The full Windows platform suite is **49/49** and the native
desktop suite is **21/21** in this checkout. These tests use only loopback
fixtures and the direct `cargo --version` child; no local model was started or
loaded.

**W9 durable fork and history path completed — still not full tree navigation.**
The Windows host now exposes the durable journal's existing runtime-fork path.
Forking before a user message creates a separately selectable child chat whose
branch ends immediately before that message. The parent remains untouched. A
new Windows CNG cursor key and a new artifact origin belong to the child, while
the storage transaction copies only the parent evidence closure authorized for
that child. The fork header records the child origin, so restart/resume cannot
accidentally claim the parent's artifact owner.

The native history panel can now page all stored journal records, retrieve the
original text of a selected user message in bounded pages, select an earlier
same-session branch point, and receive the terminal receipt expected by the
shared renderer after either synchronous navigation or a fork. Selecting a user
message moves that text back into the composer and leaves the active branch
immediately before it, matching the established AG1 tree behavior. Optional
abandoned-branch summaries remain explicitly unavailable until their
model-backed native compaction workflow is ported.

Validation includes a durable fork fixture: it verifies the child excludes the
selected prompt, the parent can still be reopened, the draft text is restored
from history, and the child compiles context after reopening with its own
origin. Current validation: storage **6 passed, 1 ignored**; Windows platform
**49/49**; native desktop **22/22**. No model process was started or loaded.

**W10 per-chat context control completed.** The native runtime already stored
AG1's selected context limit in the durable session configuration, but AG1W's
desktop command router had not exposed that capability. The Windows host now
reports the active working limit and the selected model's maximum in its runtime
view, and accepts the existing `Model/context` command only while the chat is
idle. It refuses values below 1,024 tokens or above the chosen model's declared
context window. The change calls the real native session configuration path;
it is therefore durable across a restart and is scoped to the current chat.
A new chat starts from that chat's model default instead of inheriting a prior
conversation's restriction.

Validation includes a native-host fixture for valid selection, both bounds,
restart/resume persistence, and new-chat isolation. Full validation on this
checkout is storage **6 passed, 1 ignored**; Windows platform **49/49**;
native desktop **23/23**; React **36/36**; TypeScript typecheck; Rust format
check; and a desktop binary `cargo check`. The fixtures use only loopback HTTP,
temporary files, and short direct children; no model was started or loaded.

**W11 durable abandoned-branch summaries completed.** The native tree command
now accepts the existing optional summary action instead of treating it as a
dead UI control. It reserves the real compaction fence, derives AG1's isolated
branch-summary request from only the abandoned trajectory, clears tools and
reasoning for that request, and submits it through the same loopback local
scheduler as a normal turn. The summary output and the full pre-navigation
trajectory are retained as context artifacts before the runtime atomically
publishes the `branch_summary` record and changes the selected leaf. A cancelled
or failed summary abandons its compaction fence and leaves the journal branch
unchanged.

The command returns an operation immediately, with normal live activity and
Stop support while the local model writes the summary; the shared renderer then
polls the normal terminal receipt. Validation includes a loopback local-provider
fixture which proves the summary request contains the abandoned conversation,
the durable `branch_summary` record is retained, its content compiles back into
the virtualized context, and the operation settles successfully. A second
fixture cancels an in-flight summary and proves the original branch remains
selected with no summary record published. Full native desktop validation is
now **25/25**, with platform **49/49**, storage **6
passed/1 ignored**, React **36/36**, TypeScript typecheck, and Rust formatting.
No actual GGUF model was started or loaded.

**W12 native history export completed.** The existing Session History pane's
HTML export control now works on AG1W project chats instead of reaching an
unsupported command. It writes exactly one new `.html` filename at the opened
project root through the checked Windows workspace publisher, refuses traversal
and replacement, and renders the complete current branch with a strict 8 MiB
limit. Image bytes never enter the export; an attachment reference states that
the original is still in journal custody. The export fixture proves safe output,
image redaction, filename rejection, and no-overwrite behavior. General chats
remain intentionally ineligible because they have no user-selected project
folder to receive an export.

**W13 native artifact review completed.** The Windows bridge now implements
the existing artifact catalog and paged-read protocol rather than treating the
Artifacts panel as an upload-only feature. Catalog pages are limited to 20
records, cursor positions are bounded and validated, and a turn-specific view
uses the same unique-call provenance rule as AG1's main client. A byte read
first resolves the artifact's origin through the active session's access
closure, then reads at most 32 KiB. The renderer never supplies an origin and
cannot read retained bytes after switching to another chat.

Validation includes a native-host fixture that creates a real stored artifact,
lists it, reads a bounded page, rejects a malformed cursor, then proves a new
chat cannot access the first chat's artifact. Full Rust validation after this
phase is storage **6 passed/1 ignored**, Windows platform **49/49**, and native
desktop **27/27**. The test fixture uses the local SQLite store only; no model
process, llama.cpp server, or GPU workload was started.

**W14 native workspace review completed.** The Changes pane now captures a
single immutable Git status/diff snapshot for the selected project chat and
returns it in 24 KiB verified pages. It launches only three fixed read-only
Git queries (`status`, unstaged `diff`, staged `diff`) through AG1W's Windows
Job Object; there is no shell string or model-controlled command. The child
gets an explicit small environment, disables system/global Git config,
fsmonitor, optional index locks, pager use, external diffs, and text
conversion. Output is limited to 1 MiB, cached only for the originating chat
and project, and the private spool is removed after each query.

Validation includes an end-to-end temporary Git repository fixture: it stages
a text file, asks the native host for a workspace review, and verifies the
status and staged unified diff return through the real Job-owned process path.
Another fixture verifies page/cursor and chat ownership without invoking Git.
Native desktop validation is now **29/29**. The fixture creates only a
temporary Git repository and never loads a model or starts llama.cpp.

**W15 native per-turn change receipts completed.** The Windows project tool
runner now creates an immutable before/after receipt whenever its native
`edit_project_file` or `write_project_file` tool publishes a change. It first
commits the model's tool-call message, retains the exact bytes under the
session's own output origin, then settles the receipt to `committed`,
`not_committed`, or `uncertain` after the checked workspace publisher returns.
If the file publish succeeds but receipt finalization fails, AG1W reports that
the file result is final and explicitly refuses to encourage an automatic
retry.

The existing per-turn Review UI now has native `Files/changes` and
`Files/change_read` handlers. They rebind every receipt to the active durable
branch, selected user turn, session workspace identity, and checked project
root before showing it. Diffs, before bytes, and after bytes are returned in
24 KiB verified pages; binary and oversized diffs are reported as unavailable
instead of being coerced into text.

Validation adds one Windows platform fixture that executes a real native edit
and proves its durable tool-call binding and byte snapshots, plus one desktop
fixture that writes a real receipt, renders its diff through the native review
bridge, and proves a new chat cannot inspect the old chat's receipt. Current
Rust validation is Windows platform **50/50** and native desktop **30/30**.
The fixtures use only temporary projects and SQLite custody; no model process,
llama.cpp server, or GPU workload was started.

**W16 recoverable native Undo completed.** The Review panel's existing Undo
action is now backed by Windows code instead of an unsupported bridge action.
It accepts only an active-branch receipt ID, verifies the current project and
post-change revision, records an immutable Undo intent, and moves the current
postimage to a generated sibling recovery name before publishing any original
bytes. Every move is no-replace and write-through. Once the first move has
occurred, a failure becomes a `recovery_required` outcome rather than a signal
to retry blindly. Recovery itself only restores a retained postimage into an
absent path.

The workspace fixture verifies both ordinary restoration and a race: a file
created after the postimage was displaced remains untouched, while the
displaced bytes stay in the recovery file. The desktop receipt fixture now
checks the full Review-panel status and apply route, restoring the original
text and making the receipt non-replayable. This does not add a general delete
or rename tool; it is intentionally receipt-bound Undo only. Current Rust
validation is Windows platform **51/51** and native desktop **30/30**.

**W17 native Chat folders completed.** The existing Chat folders UI now has a
Windows-native backend instead of an unsupported command. Folder grants are
per chat, persistent, identity-checked, and limited to non-overlapping roots.
Each local prompt captures one immutable grant snapshot before the model starts,
so a later permission click cannot expand that running turn. Project tools use
relative paths only in the selected working folder and accept absolute paths
only when they resolve inside another currently granted folder. An extra
read-only folder remains read-only even when the chat's per-prompt approval
permits normal project writes.

The project file list marks non-working roots with absolute paths so a model
can use them unambiguously. Search reads every granted root within its ordinary
bounds, then caps its combined result at one 32 KiB tool payload. File change review and receipt-bound Undo now re-resolve the receipt's
recorded root against the active grant snapshot; removing a folder revokes
later review or undo access rather than falling back to an old path. Changing
the primary folder changes the native file browser and direct build-command
working directory. Forked chats start with only their working root—the parent
chat's extra grants do not silently transfer.

Validation adds a platform fixture covering a read-only secondary folder,
absolute-path reading, denied writing, an outside path, and a combined-search
output cap; plus a native desktop fixture covering add/primary selection/restart
persistence. Current Rust validation is Windows platform **53/53** and native
desktop **31/31**.
The fixtures use temporary folders and the local SQLite custody store only; no
GGUF model, llama.cpp server, or GUI was started.

**W18 CV2 contract reconciliation completed.** The Windows validation record
had three failing retained web-output captures. They described an older parser
and presentation shape, not a current runtime defect. The DuckDuckGo parser
now consistently decodes entities and counts the requested number of unique
results after deduplication, including normal HTML whose attributes are in a
different order. The retained fixture now records that behavior.

The current DOM Markdown extractor, relevance selector, and empty-search
recovery deliberately add richer facts, source excerpts, scoring annotations,
and recovery suggestions. Their precise quality is covered by the dedicated
search-quality and search-recovery suites. The older captured contract now
checks the stable request/response protocol, provider attempts, and safe hint
shape without forcing those newer fields back to historical bytes. This keeps
both contracts useful instead of weakening the current implementation.

Final source validation is AG1 core **all tests passing** (with only its one
explicit live-evidence replay ignored), retained tool contract **28/28**,
Windows platform **53/53**, native desktop **31/31**, TypeScript typecheck, web
production build, and Rust format check. No model, llama.cpp server, or GUI was
started during validation.
