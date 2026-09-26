# Local image previews — 2026-09-10

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Implemented the user's image-presentation repair. This is a local-file capability and UI change, not general website screenshot support. Existing work and the installed release were preserved. No live inference or duration workload was run.

## Behavior

- Successful tool image attachments appear directly in the conversation, outside collapsed Activity/Arguments/Output. Images retain aspect ratio, are bounded to the conversation width and 420px display height, and open the existing zoom/full-resolution dialog. Expanding tool details does not duplicate the image. Failed tool images are not promoted as successful output. Draft attachment thumbnails are unchanged.
- A whole-file `read` of a simple vector `.svg` returns source text and a PNG preview automatically. No shell command, ImageMagick, website tool or web server is needed. Ranged reads remain source-only for editing. The original SVG bytes are retained in journal metadata and offered as a download-only octet-stream blob; raw SVG is never injected into the document. The underlying source file is not rewritten.
- Existing PNG/JPEG/GIF/WebP tool images also receive the larger inline presentation. Markdown's arbitrary image URLs are still not automatically fetched or rendered; images travel through authorized tools and journal references.
- Text-only models can continue after a tool returns an image. Provider projection substitutes an explicit non-vision note for the tool image, while the journal/UI retain the image. Vision-capable models still receive it. User-supplied image inputs still require vision; this change does not pretend the model visually inspected anything.
- Native system guidance and model-facing tool descriptions distinguish `read` for local images from browser tools for ready AG1-owned Runtime web targets. Saved historical catalog metadata remains preserved. Prompt validation accepts only exact saved or host-derived native metadata; altered descriptions and argument schemas remain rejected.

## Rendering boundary

Uses pinned [resvg 0.45.1](https://docs.rs/resvg/0.45.1/resvg/) with default features disabled. The parser and renderer are in-process; no subprocess, browser, system-font scan or network access is involved. Both image-resource resolver callbacks are explicitly disabled.

Preview input is bounded to 256 KiB, 2,048 XML nodes, 32 levels of nesting and a maximum 1,024×1,024 raster. DTDs, processing instructions, event handlers, external references and unsupported elements are rejected for preview. Supported elements include paths, basic shapes, groups, gradients and clipping paths. Text, use-expansion, filters, masks, scripts, foreign HTML and embedded images are not supported by this initial restricted renderer. Unsupported/oversized/malformed SVGs remain readable as source with an explanation, not a silently incomplete preview. Cancellation is checked between bounded parse/render/encode stages; individual library calls are synchronous, not hard-preemptible.

18 locked transitive crates were added; dependencies were fetched once. Builds/tests then ran offline/locked in `.local/p14-target`, with incremental compilation disabled. Existing dependency versions were not updated. The repository notice generator retained 513 exact dependency license sets; the offline notice validator passes. Existing notice bytes were checked against package sources before regeneration.

## Verification

Receipts: `.local/p14-live-fixes/image-preview-lVrE0g/`.

- 95 frontend tests across 12 files pass; typecheck and isolated renderer build pass. Added coverage for collapsed-activity image visibility, no duplicate on expansion/polling, source download bytes/content type, no SVG/iframe injection, failed-result handling and retry.
- Three SVG tests pass: real PNG pixel/color/dimension checks, original-source preservation, ranged-source behavior, bounded dimensions/nodes/nesting, rejected active/external content and cancellation.
- Seven provider request contracts pass, including all previous provider-format comparisons plus text-only continuation/vision preservation/user-image rejection/journal immutability.
- Four native prompt contracts and one exact-metadata validation unit test pass. Both folder/conversation integration contracts pass after the metadata compatibility correction.
- Default desktop check, diagnostic native build, scoped whitespace and probe syntax checks pass.
- Final diagnostic candidate: `.local/p14-live-fixes/native-candidate-QF6dsP/`; desktop SHA-256 `f7fe11cd07d4a7d1fe882242137bc06abde54ee72fdb67c8310b6ccf4925746e`; embedded renderer `/assets/index-Z6TeR2Kw.js`.
- Native image workflow passes in `images-04DVPc/result.json`: actual approved SVG creation, actual native read/rasterization, inline 320×240 image with collapsed tool output, source download to the fixture's private Downloads directory with byte equality, zoom/full-resolution/close, and 900×600 layout. One prompt, three fixed loopback responses, text-only fixture model; no real model. The desktop screenshot was visually inspected.
- Native regression probes on the same binary pass: folders `folders-1gHGq7`, approvals `approve-PEeCsH`, editor/recovery `editor-DI4Sy8`. These are private authenticated Xvfb/DOM-driven diagnostics, not physical-input or full release acceptance.
- Final image rerun also passes: `images-IMzmzW/result.json` (`image-audit-final.log`), with an additional assertion that the full image can be scrolled into the 900×600 conversation viewport. The small-window screenshot was visually inspected. Owned candidate/display processes were confirmed stopped.

## Failed attempts and remaining comparisons

- `svg-tests.log`: parallel new tests competed for the deliberate shared decoder permit. The fixtures now serialize; the exact failed case and full SVG tests pass in `svg-tests-rerun.log` and `svg-tests-final.log`.
- `native-candidate-a2L6K0/images-XWd1TX`: exposed the real text-only-model continuation failure. The provider projection fix has a permanent regression test; the full native flow now passes.
- `folder-regressions.log` and `native-candidate-TOsbVE/images-gocV1X`: native descriptions initially conflicted with exact captured metadata validation. The host-derived alternate metadata path fixes this without accepting arbitrary schemas. `folder-regressions-rerun.log` passes. The native driver also waits for the project dialog to close before querying the selected project.
- `prompt-metadata-tests.log`: new unit fixture omitted required date/cwd inputs. Corrected fixture rerun passes (`prompt-metadata-rerun.log`).
- `native-candidate-QF6dsP/images-CCLUMz`: image creation/presentation/continuation passed, but the diagnostic tried fetching a blob under the app's restrictive connect-src policy. The policy was not loosened. The replacement test clicks the real download link and verifies actual file bytes in private Downloads; it passes.
- The prior six historical comparison mismatches were rerun and persist (preview argument validation, research guidance, and source-attribution fields). The shell static-server error comparison additionally differs because its misleading local-HTML screenshot instructions were deliberately corrected here. `core-regressions.log`: 20 pass/5 comparison failures; `platform-regressions.log`: 32 pass/2 comparison failures. Frozen reference fixtures were not changed. Full-suite green is not claimed.

## Handoff

Not installed systemwide. Installed current remains `releases/a459017613ba4372a84f2dafece16dc8`, desktop SHA-256 `ea8bb4a870f9acbb62a636543ccaa6a14681b1804acfc8887e8a3b0b84f8e92f`. Pinned release process-host, model-profile, frozen runtime capture and native configuration hashes were rechecked unchanged. No user projects or frozen live-audit evidence were changed. Next step is an explicitly authorized release build/install for user testing, including the prior approval and folder controls; live inference still needs separate authorization.
