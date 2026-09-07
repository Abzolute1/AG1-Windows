# P09 tool compatibility decisions

P09 is in progress. This document explains explicit changes to selected reference
behavior; it is not a passing phase report.

## File mutation aliases and publication failures

The selected `file-mutation-queue.test.ts` permits a symlink alias to share the
target's queue. Native P03 file capabilities instead reject symlink traversal,
including a link whose target is inside the workspace. This retains the existing
descriptor-based containment boundary: resolving a link before queue admission
would not authorize its target at publication time. This is a documented capability
restriction, not a claim that the SDK alias-queue test exposed a bug. Literal paths
and normalized `./` aliases share the native queue. Rejected symlink proposals
leave both the link and target unchanged. The native platform tool contract checks
these outcomes explicitly.

Both implementations retain existing bytes when cancellation precedes publication
and preserve ordinary existing file modes. Native P03 additionally uses revision
checks and refuses copying set-ID/sticky modes. A directory target fails before
publication. Cancellation after a temporary file has been created and a parent
replacement during publication remove owned transaction files; tests inspect the
original and moved parent directories. Failed publication never reports success.

## Result externalization and evidence custody

The selected SDK `tool-result-artifacts.test.ts` exercises a standalone optional
externalizer. Native projection is integrated with P05 evidence custody: it always
retains the original result container, including small resident results. Therefore
the SDK's empty-store assertions for small/manageable results intentionally become
one original-container record and zero text artifacts. A noisy output or reused
producer has its original-container record in addition to the exact text artifact.
There is no redundant truncated text artifact when native producer reuse succeeds.

This difference implements P09's requirement to retain full original evidence,
including post-hook inputs and unknown fields. It also permits recovery without
repeating an effect. Resident small content is unchanged. Text threshold behavior,
acknowledgement/summary/image ordering, diff classification, head/tail boundaries
and exact UTF-8 paging retain their selected SDK semantics.

Native preview frames use P05-authorized cursors and native metadata under
`details.toolResultProjection`. They are not byte-for-byte SDK JSON frames. Every
frame counts against the aggregate UTF-16 budget, correcting B01. Small blocks
that collectively overflow are captured together; if multiple individual previews
cannot fit, one aggregate preview preserves the same complete text evidence.
Producer reuse additionally requires a native capability bound to the producer,
tool, call and requesting origin. Untrusted `fullOutputArtifact` JSON is not such
a capability, and a successful post-hook replacement clears the producer grant.

## Read path spellings and recovery

Home and Unicode-space expansion retain the SDK lexical behavior. A path beginning
with the exact workspace prefix but missing its leading slash is repaired before
containment. Read-only fallback order is literal, macOS AM/PM narrow space, NFD,
curly apostrophe, then NFD plus curly apostrophe. Each candidate uses P03 no-symlink
descriptor checks; the chosen filename and captured revision are bound to the
prepared read and its inspection evidence. A later literal file appearing does
not redirect an already prepared read. Writes retain their literal spelling.

The SDK checks containment before its fallback `fileExists` checks; native code
checks each candidate as well. A fallback symlink therefore fails without exposing
its target. Explicitly granted external resource roots use the same candidate
checks and still revalidate the root grant before execution.

`FileService::rejection_hint` supplies recovery context for an already rejected
lexical escape. It reports the actual workspace root and at most three matching
relative filenames, examining at most 64 directories and 4096 entries through
bounded P03 directory reads, to depth four. It skips symlinks and checks native
cancellation between entries. It does not inspect the attempted outside path.
Hints describe an observation, not permission to retry; a new call must pass the
normal preparation and authorization checks.

The deterministic tool driver supplies this hint to `executor::reject_with_hint`.
The original rejection remains an error with no execution effect or start event;
the augmented result is retained and replayed through P05. P10 should use the same
composition at its file-preparation rejection boundary.

## Bash cutoff diagnostics

The native persisted-write note recognizes both `here-document delimited by
end-of-file` and Bash's actual `here-document at line N delimited by end-of-file`
warning. The SDK matcher only handles the first spelling. An owned Bash command
with an unfinished heredoc verifies the warning, recommendation to use the write
tool, and actual bytes that reached disk. Persisted byte/line notes describe the
observed file; they do not assert that the intended write was complete.

## Native Bash custody and reusable operations

The reusable local execution surface is the native `ShellService`: callers supply
host-approved configuration and an authorized call. The source SDK's standalone
`createLocalBashOperations` and `executeBash` test cases map to this native adapter;
this does not claim those JavaScript factories are exported by the compatibility
helper. Prefixes and environment enter through native configuration and participate
in the preparation/approval binding. Missing working directories and executables
return typed native errors before a process can perform the command.

P05 retains original shell evidence even when the old standalone SDK's optional
`AG1_RETAIN_BASH_FULL_OUTPUT` flag is absent. Consequently, a successful small result
has custody metadata, and line-truncated output has an authenticated artifact
reference rather than an optional temporary `fullOutputPath`. The selected default
and explicitly enabled retention cases preserve their complete bytes in P05; the
old absence-of-retention and filesystem-path assertions are named compatibility
corrections. The 3,000-line fixture verifies the exact original stream and artifact
head/tail. This does not relax the configured output quota: overflow remains
explicitly incomplete.

Search rejections keep a typed permission reason plus native recovery context.
The deterministic driver persists both before execution and replays the same
result. The JavaScript SDK expresses the same recovery action in one thrown string.

## Sandbox configuration and launch boundaries

`Sandbox::parse` and `ShellConfig::with_sandbox_from_environment` resolve admitted
host settings, including mode normalization, explicit-option precedence and optional
command templates. They never read the ambient process environment. The native host
must pass any permitted `RUSTUP_HOME` or other toolchain variables explicitly.
`ShellService::new_with_network` can additionally disable networking in workspace
mode; strict mode always isolates networking. This policy participates in the
approval target revision.

The native launcher uses a held workspace descriptor with `--bind-fd` and direct
argv, rather than the SDK's shell-rendered `--bind path path` command. Bash runs with
`--noprofile --norc -o pipefail -c`; login startup files are not evaluated. Literal
cwd/command arguments, namespace flags, read-only root/private tmp layout, toolchain
PATH filtering and explicit toolchain environment are checked with an owned argv
fixture. A separate real Bubblewrap test checks kernel containment and pipefail;
the argv fixture makes no containment claim.

Native availability is established on the requested launch, with file availability
rechecked during preparation and execution. The SDK's cached `bwrap --version`
boolean is not a native execution grant. Removing a previously available backend
invalidates the next preparation and an already prepared execution; a present
backend that exits with an error yields an error result. The SDK's fallback to
unsandboxed execution with a warning is deliberately rejected. These availability,
cache and launcher-representation differences are explicit compatibility
corrections; namespace failure cannot authorize a weaker execution mode.

## Artifact settlement and workspace ownership

The native result projector retains the original JSON container for every result,
including explicit artifact pages. The selected SDK rehydration test expects an
empty artifact store; P05 custody intentionally changes that storage assertion.
The page content remains unchanged and creates no recursive text artifact or
replacement pointer. Exact selected large-result head/tail evidence is retained
before the preview is formed.

A successful native built-in `check` commits its completion record and releases
verification-scoped evidence leases in the same SQLite transaction. Failed checks
retain leases; extension result hooks cannot turn failure into verification.
Independent leases remain. Recovery reads the already committed receipt without
releasing newer leases. Failure injection exercises rollback, commit and missing
acknowledgement; an uncertain intent is never automatically reexecuted.

The session-owned native `FailureTracker` preserves the SDK's missing-tool result
classification and immediate shell guidance after the second missing command.
Success or explicit reset clears history, unrelated errors do not. Counts and
input sizes are bounded. P10 must observe each newly completed result once and
add returned guidance as a system contribution before bounded projection; it must
not count replayed receipts or treat guidance as verification evidence.

`workspace_lifecycle::Lifecycle` supplies native empty/active/preserved ownership
and discard for P10/P12 composition. Admission is a host capability for one child
of dedicated AG1 workspace storage, retaining its directory descriptor. There is
no model-selected deletion path. The owner must quiesce all jobs before marking a
workspace preserved. Discard refuses active state, checks cancellation and exact
directory identity, and uses P03's bounded atomic detach/cleanup. It never follows
symlinks into a source project. Replacement paths conflict. Cleanup failures retain
uncertainty and do not claim successful removal or re-adopt a new directory.
Session persistence, workspace creation/promotion and client command routing
remain the later phase composition responsibilities.
