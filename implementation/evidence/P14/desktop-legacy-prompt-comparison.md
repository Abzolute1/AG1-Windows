# Legacy initial prompt and first-response comparison — 2026-09-09

Inspected the user's `/home/alex/Projects/Cruzible` source read-only. No legacy session was opened, and no active inference/user window was controlled. The offline probe imported the compiled legacy prompt/definition builders under the network guard; it did not invoke tools. Exact results: [comparison JSON](desktop-legacy-prompt-comparison.json). Probe and private prompt diffs: `.local/p14-bypass-scope-fix/`.

## Source findings

- `packages/coding-agent/src/core/system-prompt.ts:273`: `llamacpp`, `llama.cpp`, and `local` select the compact prompt profile. The role at line 225 matches the desktop's captured role: “You are a coding assistant inside AG1. Use the named tools, inspect evidence, and answer precisely.” Neither reconstructed legacy prompt nor the two recorded native requests contains Claude/Anthropic.
- `packages/coding-agent/src/core/agent-session.ts:1851`: rebuilds using the registered active tools, their snippets/guidelines, loaded context/skills, and a workspace snapshot memoized for that session. `system-prompt.ts:256` adds current date/cwd/snapshot.
- `packages/coding-agent/src/main.ts:894` awaits `prepareStartupModelsFile`; line 576 awaits `ensureLlamaCppServer`. Interactive mode is constructed afterward at line 1012. Thus model load time is paid during startup before the TUI accepts the first message, unlike desktop's lazy first-request admission. This establishes scheduling, not a measured whole-application latency advantage.

## Bounded prompt comparison

Reconstruction uses the same 28 names as the native first request, no project context or skills, and a general-chat cwd. Four names use frozen catalog metadata because they are not direct exports under those names in the current built-in catalog (`interact_then_screenshot`, `screenshot_url`, `submit_plan`, `verify_ui`). It is not a captured live legacy request; the caller's actual selected tools, extension transformations and context may differ.

| Component | Legacy reconstruction | Rust recorded/configured |
| --- | ---: | ---: |
| Base system prompt, characters | 14,856 | 16,514 |
| Actual native system with added runtime context | not reconstructed | 17,492 |
| Same-named tool description/parameter JSON, characters, excluding host sourceInfo | 27,689 | 33,316 |

These are character counts, not token counts or wire-payload equivalence. The actual native server measured 13,421 first-prompt tokens. Both harnesses include substantial tool guidance; a claim that only Rust has a large tool prompt would be unsupported. Rust has additional overhead in this comparison, but it does not establish the size or timing of the user's earlier legacy request.

The saved desktop `system_prompt` includes `Current date: 2026-09-07`, `Current working directory: /home/alex/Projects/Cruzible`, and that repository's top-level listing. It also names tools such as `check` that are absent from the recorded active tools. The native session already has a workspace/resource/registry prompt builder (`product/crates/ag1-platform/src/session/prompt.rs`), but the captured configured system text feeds its custom-prompt path. That stale snapshot is a configuration defect, not appropriate general-chat context.

## Actual running model and timings

Live process 497705 loaded the Qwen3.6-35B-A3B-UD-Q4_K_XL GGUF from the nonMTP model directory. `/props` agrees on path/alias, and `/v1/models` exposes that exact ID. No evidence supports its generated claim to be Claude. The actual requests do not instruct it to claim that identity; internal causation cannot be inferred from the logs.

The native server log shows 22.872 seconds to loaded/listening, then 34.873 seconds prefill and 1.587 seconds generation for the first request. The second has 0.594 seconds prefill plus 1.337 seconds generation, with prefix reuse. Server load timing excludes any preceding host verification, and this is not a matched legacy/Rust benchmark.

## Follow-up work, not implemented by this investigation

1. Replace only the imported captured base prompt with the current workspace/active-tool builder, preserving deliberate user custom prompts and durable request provenance.
2. Prepare the explicitly selected local model before Send with observable loading/ready state, cancellation and strict selection/custody checks; preserve the pinned launch tuning. Avoid silently loading a fallback.
3. Audit prompt/schema overhead with matched initial payloads and measured prefill; do not remove capabilities or weaken runtime policy merely to reduce token count.

The separate bypass-lifetime correction was installed and tested; see [its receipt](desktop-bypass-scope-fix.json). No system-prompt or runtime-startup changes were made in that installation.

## Implemented follow-up — 2026-09-09

The prompt refinement, exact captured-default migration, preserved user/project append instructions and tokenizer comparison are now implemented. See [prompt and image delivery](desktop-prompt-images.md) for current results and limits. Earlier observations above describe the investigated build.
