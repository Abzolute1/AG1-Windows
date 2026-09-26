# Hide retained-output chat footer — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified: `/opt/ag1/releases/743f2534917f4ab1b6bcd25fc6f79a85`.

Removed the TurnOutputs mount/import from NativeConversation and left a temporary-hide comment. The component remains available for later restoration. Retained artifacts, Artifacts review and file-change cards are unchanged. This also avoids the footer's artifact-list requests.

13 existing conversation tests passed. Frontend typecheck/build, native release build, archive validation and all 1,144 installed payload files/receipt verified. Protected configuration unchanged. No desktop capture. Restart AG1 for the new UI. Scoped patch and logs: `.local/hide-turn-outputs-20260911/`.

[Machine evidence](../../P14/hide-turn-outputs-20260911.json).
