# Startup progress restored — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed release: `/opt/ag1/releases/735843f2eba04672926c63a301797417`.
Restart after the current audit finishes to use this UI repair. No running user model or desktop process was stopped.

The Qwen3.6 backend was already working: its owned llama-server log showed prompt processing advancing at approximately 300 tokens/second, and the user subsequently observed output. The compact workflow refactor only mounted its header after durable assistant/tool history existed. Before the first output, the chat looked blank despite the existing host loading/waiting observations.

The active turn now mounts its workflow header immediately, including elapsed time and the existing host status (verification, loading, preparation, waiting, thinking, execution and stopping). Startup has no misleading zero-tools counter or empty dropdown. Status remains visible when work details are collapsed. Terminal failures still suppress stale activity through the prior stream repair.

45 focused frontend tests passed, including a new user-message-only startup regression through loading/waiting/thinking/stopping and completion. Production frontend/native builds and package validation passed; isolated desktop and narrow-width browser checks showed the timer and loading status without overflow. No user desktop capture. All 1,144 installed files, receipt and protected hashes verified.

Approval follow-up: the supplied screenshot explicitly showed “Bypass approvals · next response”; the audit started with Ask before changes. The workspace had no persisted approval override and used the initial interactive policy. Native policy code permits normal tools under Disabled/Bypass; policy changes are applied at next-message admission. No bypass defect was established in this run, and active-turn approval semantics were not changed.

Private evidence: `.local/startup-progress-20260911/` (incident.json, implementation.patch, tests.log, visual.json, frontend-build.log, package.json, installation-verified.json). Temporary preview entrypoints removed. Weekly usage snapshot: 2% remaining. Broader backlog remains paused.
