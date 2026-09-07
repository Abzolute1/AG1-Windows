# Isolated native clients

P12 clients compose `ag1-core`, `ag1-storage` and `ag1-platform`. React, the CLI and the SDK send the same versioned commands. The Rust host owns the task, journals, approvals, provider scheduler and processes. Ordinary desktop authority remains the selectable P04 TypeScript worker until P13.

Build from the repository root:

```sh
cargo build --manifest-path product/Cargo.toml -p ag1-cli --offline
product/target/debug/ag1-cli --init /absolute/new-isolated-project
product/target/debug/ag1-cli --workspace /absolute/new-isolated-project \
  --state /absolute/new-private-state --config /absolute/host.json --print 'Review this project'
```

`--init` requires a new directory. Opening requires its `.ag1-native-isolated` marker and a separate private state directory. Do not point these clients at the original AG1, runtime trees, weights or user configuration. The state has its own writer lock, session selection, artifact custody and private resource home. Native desktop mode is fixed at startup with `AG1_NATIVE_CONFIG=/absolute/host.json`; `AG1_DESKTOP_STATE` selects its private catalog and per-project native states. Switching projects drains owned work. No model generation starts on opening settings, mounting React or selecting a model.

A minimal fixture configuration is:

```json
{
  "models": [{
    "provider": "fixture", "id": "initial", "api": "openai-completions",
    "baseUrl": "http://127.0.0.1:19999", "contextWindow": 32768,
    "maxTokens": 4096, "input": ["text"], "reasoning": false
  }],
  "selected": "initial", "credential_env": null,
  "system_prompt": "Review this isolated project.", "fixture": true
}
```

The fixture endpoint must be a fake server owned by the caller. The acceptance harness creates its own server on an ephemeral port; it never contacts a live model. `fixture` labels evidence and is not an endpoint-ownership grant. A production host supplies its admitted model catalog and credential environment bindings. P13 owns transfer from the existing TS/live runtime owner. Never run these isolated clients concurrently with another owner of a local inference endpoint.

Model metadata uses the P06 API families and request adapters. `credentials` maps provider IDs to environment-only host bindings: `api_key`, `bearer`, `gemini_cli` and `aws` (see `ag1-platform/src/client/credentials.rs`). Credentials are resolved in Rust. They are absent from client projections and HTML exports. OAuth exchange, refresh and custom-provider broker APIs remain the P06/P09 host APIs; this CLI does not substitute a browser login or read the user's credential vault. Runtime metadata and `runtimeConflict` are displayed; selecting a conflicting model fails. Working context limits are durable core configuration, not llama tuning. No client command rebuilds an engine, chooses a stock fallback or rewrites a tuned profile.

Optional `tooling` supplies absolute `process_host`, `git`, `rg`, `fd`, `bubblewrap`, and `targets` paths. Targets have optional `node`, `npm` (the npm CLI script), `cargo`, `rustc`, `godot`, and explicit read-only `mounts`. An optional `browser` has `node`, `driver`, `playwright`, `executable`; it enables the P11 screenshot/interaction tools against an owned ready preview. Current model vision is revalidated before execution. Missing capability is an error with a reason. Checks and previews share the same runtime catalog. Native embedding and judge-only vision are unavailable in this binding.

Optional `extensions` supplies `node`, `host_script`, `process_host`, `trusted` and `allow_inference` (default false). Trust is explicit. Only captured isolated workspace/private-home resources are loaded. Explicit extension commands may use the bounded native UI bridge; permitted inference uses the same Rust transport and scheduler as the parent. P10 native lifecycle/tool/request callbacks retain DenyBridge and static registration fences. See [compatibility](COMPATIBILITY.md) for those boundaries.

Optional `packages` supplies `process_host`, `node`, `npm_cli`, `git` and `offline`. Installation/update/removal runs through P09 policy, one-use approval and execution custody. Enabling/disabling project package resources is a separate approval-bound settings CAS; unrelated settings and filters survive. Local packages can be enabled directly. Installed user packages are discoverable through the private host's settings; no client writes the actual user home. A newly constructed session rebuilds the admitted resource/tool catalog.

## Modes and receipts

`--mode print` (`--print`, `-p`, `--mode text`) uses native print completion policy. Output is the complete final assistant text, with a nonzero exit on failure. Headless policy does not silently grant a pending interactive approval; `--trust-workspace` explicitly selects autonomous policy. `--max-turns` is an optional caller limit; there is no implicit parent turn cap.

`--mode jsonl` (`--json`, `--rpc`, `--mode json`, `--mode rpc`) accepts generated `WireMessage` Command and Cursor frames on stdin and writes only frames to stdout. Diagnostics go to stderr. The first frame is a native Client view containing version, scope and revision. Copy these into each CommandEnvelope, assign a unique command ID and retain it for recovery. Frames are bounded at 64 KiB. Malformed lines produce protocol errors; an oversized unterminated line stops input consumption. EOF drains accepted work; EOF with pending approval cancels owned work and fails. An explicit Cancel targets the returned operation ID. Closing a subscription/pane never cancels native work.

Other supported adapters: `--workspace`/`--cwd`, `--state`/`--agent-dir`/`--session-dir`, `--config`, `--provider`, `--model`, `--thinking`, `--tools`, `--no-tools`, `--session`/`--resume` (native ID), `--continue`/`-c` (durable selected session), `--fork` (user entry ID), `--system-prompt`, `--append-system-prompt`, `--export` (new HTML path), `--list-models`, `--version`, `--help`, and `--`. Unknown flags fail explicitly. These are native adapters, not identical legacy path/resume selector semantics.

Native session commands retain their existing command result/replay contract. Client effects (save, export, project lifecycle, terminal input and upload/UI answers) additionally retain a native intent and immutable reply. Duplicate acknowledged effects return the reply without repeating the effect. An intent without a receipt is uncertain and cannot be automatically replayed. `runtime.client_effect_result` with the original `command` ID inspects it in the same workspace/session, including after reopening. Inspect actual state before a new action; changing the ID is not a recovery procedure. File saves also require the exact file revision; exports create new files only.

Histories, original entries, artifacts, detail records and process logs have bounded pages. Artifact reads verify custody and integrity. A shortened display is labeled and does not replace the original record. HTML export includes complete active-branch text and original tool JSON up to an explicit 8 MiB file limit, references attachments retained in the journal, escapes content, forbids scripts and retains the AG1 MIT notice. Exceeding that limit is an error, not a partial successful export.

Vite preview readiness matches the complete served project package.json (up to 8 KiB), the exact loopback address and an owned listener. Other custom dev scripts or larger manifests can run but do not receive an automatic HTTP preview grant. Terminal target completion is drained before the next install/build/run; stopping or switching invalidates the preview.
