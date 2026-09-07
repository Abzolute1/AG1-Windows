# P13 — blocked before authority cutover

P13 has **not passed**. No real local migration, authority transfer, or live model canary was performed. The previous usable owner remains available. No phase completion commit was made and P14 was not started.

## Verified entry

The starting worktree was clean at `6431a35`. All transitive dependency artifact manifests verify, including the 1,536 artifacts in P12's passed certificate. Current original source pins, the tuned profile, binary/library fingerprints, 30 weight files and loader material verify using the existing preservation gates. Earlier delivered snapshots and certificates remain unchanged.

The supplied handoff correctly describes the source: ordinary desktop startup still uses the TS bridge; native clients require isolated workspace admission. P13's authority-transfer implementation, desktop `cutover_contract`, and phase gate do not yet exist. Explicitly invoking the two required commands confirms they fail instead of returning a placeholder pass. These are outstanding implementation requirements, separate from the live runtime blocker.

## Runtime window blocker

Read-only inspection of the actual host found pre-existing llama-server PID 1311504, parent PID 1, started September 4, serving the tuned Qwen3.6 35B model on port 18080. Its process identity and argv remain unchanged after fixture verification. No owned process handle, custody token, or exclusive scheduling grant for this service was established. GPU inspection reported 15,361 MiB used and 610 MiB free on the RTX 4080. The sandbox's private process namespace does not expose this host service; the recorded host inspection is authoritative for this observation.

The user explicitly prohibits stopping externally owned services and changing tuning. Therefore this execution cannot obtain the exclusive runtime window for the required three sequential TS/Rust model comparisons. No endpoint request, model launch, stop, retune, rebuild, substitution, or live ownership adoption was attempted. All three canaries are `not_run`, not passed. Readiness, an idle endpoint, or port availability would not establish ownership.

Resume requires the external owner to release the service/resources and an exclusive runtime window to be established under the original preservation constraints. Merely rerunning this phase does not authorize evicting that service.

## Fixture verification and current-source correction

The current P05 storage contract passed **39 tests**, including frozen import, repeated interrupted import, one writer, raw/unknown record preservation, lossless export to a new directory and restore, compatible export, unsafe downgrade rejection, corruption, artifact/cursor closure, and abrupt process-exit recovery. This rehearses the existing storage migration/rollback primitives in disposable fixtures. It does **not** certify the missing P13 transfer state machine or ordinary-workspace rollback.

The current client contract passed **23 tests** after correcting one test synchronization race. The JSONL shutdown test previously called `try_wait` once immediately after stdout EOF and failed if process exit status was not yet visible. It now waits up to ten seconds for that distinct event while retaining exit-success and protocol/content assertions. No product runtime behavior changed. The original sandbox attempt and first host rerun remain in separate logs: sandbox restrictions blocked fixture listeners/processes, then the host rerun exposed the EOF/exit race. Neither failed attempt is counted as passing. Formatting passes.

## Preservation and remaining work

The profile SHA-256 remains `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`. Original user data, configuration, recovery archives, tuned servers, adjacent libraries, weights and projectors were preserved. No user migration destination was created. Entry and exit preservation records and the external runtime observation are retained alongside the logs.

Mandatory work still pending: durable authority marker/lease and transfer/recovery protocol; startup and every admission-path fence; read-only configuration import with exact launch/request resolution; complete transfer manifest and rollback closure; P13 crash/race/tamper/client-receipt/preview contracts; migrated full workflow; actual new-destination user bundle migration; all three exclusive sequential per-model canaries; measured cutover/recovery evidence; and a passing P13 phase gate. The storage and client regressions cannot replace these gates.

The only source edit is the bounded shutdown wait in `product/clients/cli/tests/client_contract.rs`. It and this blocked evidence are left uncommitted because the requested commit is conditional on P13 passing.
