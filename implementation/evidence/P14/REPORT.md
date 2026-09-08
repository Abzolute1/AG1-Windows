# P14 release execution — work in progress

P13 entry was verified read-only before implementation: every prior artifact
manifest passed, the current product fingerprints matched accepted P13, the profile
and bound process supervisor hashes matched, and the selected native authority was
active, quiescent, at epoch 2. P01–P13 reports and private recovery archives remain
immutable. `accepted-entry.json` retains the accepted fingerprints separately.

P14 is **not accepted**. `completion.json` is authoritative for the latest executed
gate. Unavailable or unexecuted live, duration, signing, platform, provenance and
matched packaged performance requirements remain named gates.

## Delivered local work

- A real Linux Tauri/React/Rust candidate and separate extension archive, built in
  `.local/p14-target`, with a payload manifest, conservative CycloneDX dependency
  inventory, retained notices and observed system shared-library closure.
- An explicit local review installer with versioned payloads, atomic activation,
  retained previous versions and interruption receipts. It rejects bad archive
  hashes, traversal, symlinks, protected runtime/state paths and unknown pending
  operations. No production state or runtime configuration transition is performed.
- `release-differential`: fresh execution of the unchanged context/runtime reference
  probes and the full native workspace tests, with outputs under P14 only.
- A P14 gate that runs actual checks and packaging and refuses absent, stale,
  undersized, incomplete or failing measurement reports. External report validators
  cover duration, live pairs, platform cases, performance, provenance and coverage.
  These schemas do not produce the missing workloads or establish their execution.
- A fresh strict live-canary driver using the existing authenticated native runtime
  owner, shared runtime lease and original exact-output validators. Its 36-case
  scope is separate from the broader release quality requirements.
- A frozen-prompt context-intelligence replay using the original benchmark prompt
  assembly and answer/evidence verifiers. Both language adapters receive both
  fixture variants on each preserved model; variant labels are not languages.
- Installed production-shell crash/restart and five-minute process measurements on
  an owned private software-rendered display. No user workspace is opened.
- All 127 planned A/I/B/L requirements and 44 child families retained in
  `requirement-inventory.json`, with inherited compatibility boundaries explicit.

## First matched result

`core-performance.json` retains three alternating TS/Rust runs, 25 samples per
workload per run. The existing comparator checks equal input, selected evidence,
ordering, output hashes, artifact bytes/counts and raw statistics. All nine pooled
p95 comparisons were within the 10% regression budget.

| Context operation | Rust p95 | Legacy TS p95 | Rust / TS |
| --- | ---: | ---: | ---: |
| 32k pressure, initial compile | 1.206 ms | 2.217 ms | 0.544 |
| 32k pressure, warm compile | 0.031 ms | 0.190 ms | 0.162 |
| 2M externalized tokens, initial compile | 24.135 ms | 28.845 ms | 0.837 |
| 2M externalized tokens, warm compile | 0.105 ms | 0.382 ms | 0.275 |

The warm-path improvement is larger than the observed per-run variation. Smaller
initial-compile differences need more controlled follow-up. These are equivalent
volatile artifact callbacks; durable Rust custody timings are separately recorded
and are not compared to unsynced TS storage. Shared workstation load was present.
These measurements do not certify inference throughput, task quality, complete
packaged performance or a long-duration memory slope.

The installed empty-home shell crash/restart smoke passed before and after a
polling fix. Five-minute observations after a 60-second warmup measured idle CPU
at 0.711% and 0.166% of one core, respectively, including descendant processes and
with no leaked owned children. The fix schedules home polling once per second,
keeps legacy/native session polling at their existing intervals, and prevents
overlapping requests. `shell-smoke-before-polling.json` and `shell-smoke.json`
retain the exact installed payload hashes and raw private observations. This
clears the empty-home CPU threshold under software rendering, but remains an
unmatched smoke measurement, not full packaged performance acceptance.

`shell-pair.json` records one completed sequential comparison of the installed
AG1 Tauri shell against the pinned original Electron Dyad shell. Each used a
1440×900 private authenticated X11 display, software rendering, isolated network,
no GPU devices, 60-second warmup and 300-second measurement. All descendant
processes were included; both process trees stayed stable and leaked no children.
Mean PSS was **274.43 MiB AG1 versus 478.40 MiB Dyad (42.64% lower)**. Idle CPU
was **0.156% versus 2.409% of one core (93.54% lower)**. Screenshots confirmed
both empty-home windows rendered on the private displays. Shared workstation and
YouTube load remained; this single pair does not certify repeated startup, display
latency or the full performance gate. The historical private driver is retained;
`tools/release_shell_pair.py` generalizes its output destination for future runs.

The product combines a Dyad-derived UI/design with AG1 machinery ported to Rust.
Original Electron Dyad is the shell baseline; legacy AG1 TypeScript is the model
adapter/behavior baseline. These comparisons measure different parts of AG1.

The latest completed stable offline replay ran 826 native workspace tests.
The updated frontend passes 18 tests; release validators and installer pass 13;
the quality example passes three validation tests and Clippy with warnings denied.
Earlier replays with changed whole-run fingerprints remain excluded. The final gate refreshes P14 evidence after source changes settle; its exact
commands, fingerprints and pending requirements are in `completion.json`.

The actual Linux tar payload also passed interruption exercises at all four
installation boundaries: pending, verified, published and activated. Each run
started from a complete installed candidate whose CLI executed successfully;
after interruption the previous payload remained hash-correct and readable,
pending receipts remained available, and an unknown retry was refused. Profile,
three tuned executable and active supervisor hashes remained unchanged.
`installed-update-interruptions.json` retains the driver, exact candidate, raw CLI
logs and receipts. These are Linux payload/update results, not production agent
state rollback or cross-platform certification. An additional installer regression
checks archive replacement between preliminary verification and extraction.

The first completed context-intelligence model comparison scored 6/6 for each
adapter on Qwen3.6-35B-A3B-UD-Q4_K_XL, with zero stale and unsupported claims.
`coherence-first-model.json` retains this interim result and the unchanged verifier's
raw observations. This is a tie under identical assembled prompts, not evidence
that the model became more coherent in the Rust session harness.

On the second model, Qwen3.8-27B-UD-Q4_K_S, TS scored 6/6 and Rust scored 5/6.
The Rust-side `revision-aware-memory:baseline` answer lacked its final closing
brace, so the unchanged JSON verifier rejected it. Both sides reported a normal
stop after 30 output tokens, below the unchanged 2048-token ceiling; payloads
matched, including the preserved temperature 0.7. Both had zero stale/unsupported
claims. This is an observed quality failure, not a confirmed causal adapter bug:
raw wire chunks were not captured, so stochastic generation and transport cannot
be conclusively distinguished. `coherence-failure-analysis.json` retains the
specific evidence. No settings change or passing retry replaces the failure.
The third model scored 6/6 for both adapters. The completed run totals **TS 18/18,
Rust 17/18**, with zero stale and unsupported claims on both sides. All 36 requests
executed, all 48 runtime events settled, and final preservation verification
passed. Native execution took 1642.371 seconds; the wrapper then exited nonzero
for the actual quality failure. `context-intelligence.json` retains the complete
scores, exact inputs, raw responses and original verifier results. Inputs remained
unchanged throughout the live run. Later release reporting and generalized shell-driver changes are itemized
in `post-quality-source-review.json`; provider/context/model code stayed identical.

The final local review package is indexed by `package.json`. The pre-gate review
candidate retained byte-identical binaries from the shell measurement, as recorded
in `post-shell-package-review.json`. The final gate then rebuilt the frontend and
desktop: `final-package-rebuild-review.json` records changed desktop binary bytes
and package manifest despite unchanged source fingerprints. The cause remains
unresolved; shell observations bind their earlier measured package, and final
binary performance/reproducible builds are not certified. The logical payload
sizes stayed equal. `package-footprint.json` inventories the pre-gate candidate's
actual files, including its manifest: approximately 71.18% fewer payload bytes than
pinned Electron Dyad. External system libraries, optional runtimes and model
material remain separately scoped; this is payload size, not a complete machine
installation comparison.

## Preservation and runtime window

The first fresh full production audit refused admission after 287.240 seconds
because a new external Discord GPU process occupied the runtime window. This
failure is retained in `entry-audit.json`; it is not a preservation pass.
The user then expressly authorized stopping Discord for testing. The current app
parent was identified through executable and process ancestry, signaled through a
verified pidfd with SIGTERM, and exited gracefully; no force was needed. Its GPU
process exited too. `runtime-window.json` references the private identity/exit
receipt. Historical P13 stop authorization was not reused.

The second full audit passed in 583.901 seconds (`entry-audit-2.json`). The first
strict-canary wrapper attempt also obtained a successful audit, then refused
before generation because its expected digest used the containing artifact's
SHA-256 instead of the native canonical report digest. The wrapper now obtains
that digest through `verify-canaries`; the failed attempt remains private evidence.
The second strict matrix passed all 36 checks with 48 runtime events, settled
owned children and successful final preservation verification. `fresh-canaries.json`
retains the complete admission evidence; the broader quality replay finished with
the failure described above.

The unchanged profile SHA-256 is
`aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`.
The active supervisor remains at `product/target/release/ag1-process-host`, SHA-256
`e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c690b06232c36bc1da2`.
All three tuned executables, loader libraries, weights, projectors, exact ordered
plans and user profile are external read-only dependencies. Candidate installation
does not bind its supervisor into the selected native configuration.

P13's native resume and authority audit include complete material/archive hashing.
They remain separate from shell readiness, model loading and usable-session
readiness. Mandatory identity checks remain enabled. P13's 6 ms renderer thaw
observation is not used as a startup baseline.

Read-only `/proc` observation during the fresh canary admission recorded more than
200 GB read before model launch (`admission-io-observation.json`). Code inspection
shows fresh capture verification and a separate runtime-host material verifier;
their independent initial verification contributes repeated material reads.
The host already caches verified identities within its lifetime using device,
inode, size, mtime and ctime. This observation identifies verification reuse as
an investigation target; no cross-owner cache or verification shortcut was added.

The first Dyad preflight followed the inherited Wayland session, leaving its
private-X11 screenshot blank; it is excluded from shell measurements. Its startup
also created a global Git safe-directory entry and an empty `dyad-apps` directory.
The exact newly created contents and birth times were checked, retained privately,
and those two additions were removed. Subsequent Dyad launches use explicit X11,
unset Wayland/session-bus settings, a disposable home, original files mounted
read-only, private temporary storage/network and no GPU devices. The second
preflight failed because Chromium could not create its temporary socket directory;
the third rendered correctly on the private display and settled all owned children.
No preflight is used as a five-minute or startup measurement.

`runtime-coexistence.json` records an ordinary-use limitation discovered while
answering the user: `client/local_runtime.rs` enables the same exclusive-GPU guard
used during benchmarks. The guard rejects any other PID reported by NVIDIA's
compute-process query, so Discord can block normal local-inference admission too.
This is an implementation restriction, not an inherent llama.cpp requirement.
No unowned stop authority or relaxed runtime policy was introduced. Ordinary app
coexistence needs an explicit tested policy separate from benchmark isolation.

## Reproduction and platform prerequisites

From this checkout:

```sh
python3 tools/phase_gate.py P14
python3 tools/reference_oracles.py --suite release-differential --offline
python3 tools/release_measurement.py
python3 tools/release_package.py
python3 tools/release_smoke.py
python3 tools/release_shell_pair.py .local/p14-shell-pair/NEW_SHELL_OUTPUT
python3 tools/release_canaries.py .local/NEW_P14_CANARY_OUTPUT
python3 tools/release_quality.py .local/p14-quality/NEW_QUALITY_OUTPUT
```

The canary command requires a fresh successful authority/preservation audit and
exclusive runtime availability; it creates private evidence and never retries an
existing destination. Its native owner enforces the runtime lease independently
of saved reports. Do not run it alongside another live workload.

Only Linux x86_64 Rust tooling is installed. Windows/macOS runners and signing
identities are not configured. Linux `dpkg-deb` and `rpmbuild` are not on PATH.
The current deliverable is a local tar candidate; native OS installers remain
pending. Exact native-runner build/validation commands for the next packaging step
are below. They are **not executed results** and do not imply that Unix-only
implementation paths have been ported.

Linux:

```sh
CARGO_TARGET_DIR="$PWD/.local/p14-target" npm --prefix product/apps/desktop run tauri -- build --bundles deb --config '{"bundle":{"active":true}}'
dpkg-deb --info .local/p14-target/release/bundle/deb/AG1_0.1.0_amd64.deb
dpkg-deb --contents .local/p14-target/release/bundle/deb/AG1_0.1.0_amd64.deb
gpg --detach-sign --armor .local/p14-target/release/bundle/deb/AG1_0.1.0_amd64.deb
gpg --verify .local/p14-target/release/bundle/deb/AG1_0.1.0_amd64.deb.asc .local/p14-target/release/bundle/deb/AG1_0.1.0_amd64.deb
```

macOS, on a native runner with the signing identity and notary profile supplied:

```sh
CARGO_TARGET_DIR="$PWD/.local/p14-target" npm --prefix product/apps/desktop run tauri -- build --bundles app,dmg --config '{"bundle":{"active":true}}'
codesign --verify --deep --strict --verbose=2 .local/p14-target/release/bundle/macos/AG1.app
xcrun notarytool submit .local/p14-target/release/bundle/dmg/AG1_0.1.0_aarch64.dmg --keychain-profile AG1_RELEASE --wait
xcrun stapler staple .local/p14-target/release/bundle/macos/AG1.app
spctl --assess --type execute --verbose=2 .local/p14-target/release/bundle/macos/AG1.app
```

Windows PowerShell, on a native runner with Windows SDK SignTool and a certificate:

```powershell
$env:CARGO_TARGET_DIR = "$PWD\.local\p14-target"
npm --prefix product/apps/desktop run tauri -- build --bundles nsis --config '{"bundle":{"active":true}}'
signtool sign /fd SHA256 /sha1 $env:AG1_SIGNING_CERT_SHA1 .local\p14-target\release\bundle\nsis\AG1_0.1.0_x64-setup.exe
signtool verify /pa /all /v .local\p14-target\release\bundle\nsis\AG1_0.1.0_x64-setup.exe
```

Artifact names/architectures must match the retained runner output. Shell readiness
is separate from inference support: the tuned Linux model builds do not establish
Windows/macOS inference compatibility, and no replacement builds are authorized.

## Remaining acceptance

Both actual 8h and 24h repeated-session workloads remain required, with 2–3M
externalized tokens, source edits, branch/fork/resume, renderer detach/reconnect,
model/host crashes, corrupt-state recovery, raw memory observations and unchanged
quality verifiers. The duration validator is not an executed workload. The
context-intelligence provider replay is implemented and failed quality parity as
described above. Required duration workloads have not started; the observed
quality failure prevents advancing release acceptance under the common phase
contract. Their workload drivers also remain unfinished.
Full-session long-horizon/repository-context/speed-probe adaptation and evidence
remain unfinished. Strict canaries and frozen-prompt provider replay cannot
replace the full-session quality and duration requirements.

Repeated matched packaged baselines, 20-launch startup milestone distributions,
display-lag distributions, complete core retrieval/persistence comparisons,
cross-platform process/protocol/preview/installer cases, signing and final exact
binary provenance review remain required. The current SBOM is explicitly
conservative rather than exact static symbol-level closure.

Keep the accepted native owner, original Cruzible/Dyad pins, all P13 private
attempts, source/state archives and the executable reference. Retain native
embedding, judge-only vision and dynamic provider rebinding limitations. Any real
state rollback needs a fresh current quiescent archive and a new compatible TS
destination; an old checkpoint grants no authority and does not undo source edits.
All L01–L08 requirements remain in force. No public release is authorized.
