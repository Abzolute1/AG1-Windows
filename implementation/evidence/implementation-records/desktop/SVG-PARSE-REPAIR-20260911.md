# SVG XML error recovery — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified: `/opt/ag1/releases/27642db93e144fa9b80010023bba8cc9`.

The user's latest SVG preview failure was malformed XML, not artwork complexity. Session `chat-837c9c5e8debaf698488a7f051a565f1`, result ordinal 23, preserved a 17,156-byte SVG with an unmatched closing `</g>` at line 360. The generic parser error discarded this distinction and caused the model to attempt unnecessary simplification. The actual working file was in the general-chat directory, not the earlier COD_Z project.

Parser failures now include the underlying XML diagnostic and exact line/column, with instructions to fix the syntax and preview again. XML node, attribute and namespace limits are distinguished as LimitExceeded. The preview fallback no longer sends all failures toward PNG conversion; it directs syntax errors back to editing the SVG. No renderer limits were raised or artwork features removed.

Five SVG tests passed, including malformed XML versus genuine node exhaustion. A private copy of the actual failed source rendered successfully after deleting only the unmatched closing tag: 152 elements, 800×600 PNG. The native ui/read/renderer/Qwen vision payload probe passed, and the rendered image was inspected. No new model inference was run and no claim is made about whether a model will always follow the recovery instruction.

Production build, archive validation and all 1,144 installed file/receipt checks passed; protected configuration hashes unchanged. User artwork and running processes untouched; no desktop capture. Restart AG1 for updated diagnostics. A corrected private copy and PNG are retained in `.local/svg-parse-repair-20260911/`, alongside the failing-source snapshot, scoped patch, tests and probe source. Weekly usage now shows 1% remaining.

[Machine evidence](../../P14/svg-parse-repair-20260911.json). This supersedes the previous image-preview release; broader work remains out of scope.
