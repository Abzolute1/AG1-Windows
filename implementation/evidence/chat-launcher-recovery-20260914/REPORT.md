# New-chat NotFound recovery — 14 September 2026

Fixed the reported failure before message submission. The installed search-quality release remains `/opt/ag1/releases/7dbee4adb4ba4f1386b96a1a96809573`; no application source change or rebuild was required.

## Cause

The desktop's saved `native-host.json` still pins its model process supervisor to `/opt/ag1/releases/4428d311dd3a40a29986e7794c422b96/bin/ag1-process-host`, with SHA-256 `f60b7d4bd3e6ff9694335bb4e64653a3e3b148341fe52733af6ef2a8d314f63a`. The earlier [storage cleanup](../storage-cleanup-20260913/system-cleanup.json) removed that release even though the desktop runtime still depended on this file.

New-chat opening validates the pinned supervisor before submitting a message. The absent file produced `NotFound`. Desktop startup already refreshes the separate tool helper to the current package; it deliberately preserves the separately pinned model supervisor. Checking only current package files and the repository's other protected supervisor failed to detect this missing dependency. Previous search-quality model probes used that other captured configuration and therefore did not exercise this exact desktop reference.

## Repair

Located the original supervisor in `.local/p14-live-fixes/native-candidate-ehxYNY/ag1-process-host` and independently verified its exact hash against the desktop configuration. Restored those identical 2,098,504 bytes to their original pinned path, root-owned and executable with mode 0755. The historical release directory now contains the recovered dependency, not a reactivated old application release.

No tuning or configuration migration was needed. The desktop configuration, imported settings/credentials, model profiles, captured runtime/launch configuration and repository's protected supervisor all retain their pre-repair hashes. The llama.cpp backend, weights and launch flags were not edited. User conversations and the existing draft were not modified.

## Verification

1. Reproduced `ag1: NotFound` with the installed CLI opening an isolated workspace using a copy of the actual desktop configuration. The copy only rebased the ordinary tool helper, matching desktop startup behavior; its pinned model supervisor and captured tuning were preserved.
2. After restoring the exact missing file, the same startup/export operation exited 0.
3. Sent a real prompt through the installed native session using the actual configured 35B model. It returned exactly `AG1_SEND_RECOVERY_OK`, exited 0 and shut down its owned runtime. Elapsed: 26 seconds. The probe used isolated conversation state and did not submit the user's retained draft.
4. Rechecked all protected configuration hashes, restored helper hash/permissions and the unchanged current-release pointer.

[Before startup log](startup-before.log), [after startup log](startup-after.log), [real model result](live-send.json), [model output](live-send.stdout), [verification](verification.json), [recovery source and destination](recovery-plan.json), [user-provided screenshot](reported-error.png).

No screen was captured. The native startup/send path was exercised through the installed CLI; the user's live GUI button was not automated. Because the saved path is restored in place and failed opening does not retain a Host, the existing desktop can retry sending without a configuration change.

## Prevention

Future release cleanup must preserve every release containing a still-referenced pinned supervisor, including the desktop's `runtime.process_host`, migrated workspace host configurations and retained runtime captures. Keeping only current/previous application releases is insufficient. Never silently replace a pinned runtime supervisor with a different binary or change tuning to work around an absent dependency.

The broader CLI research-completion finding in the [search-quality report](../search-quality-20260914/REPORT.md) is separate from this restored desktop startup dependency.
