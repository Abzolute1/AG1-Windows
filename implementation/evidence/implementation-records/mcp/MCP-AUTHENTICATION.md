# MCP manual credentials — installed checkpoint, 2026-09-12

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

This is the historical C1 checkpoint. The current installed build includes [C2 OAuth public pre-registration](MCP-OAUTH-PROGRESS.md); C1 evidence below is retained unchanged.

The [C1 manual-credential checkpoint](../../MCP/authentication/manual-acceptance.json) is verified and [installed as a normal per-user production distribution](../../MCP/authentication/manual-installation.json). Restart AG1 from the application menu to load it. **OAuth and the full phase C/A–F integration are still unfinished.**

Users can enter remote bearer tokens or named local environment values in a GTK dialog outside WebKit. Saving private values retires affected server connections across scopes; activation and operation approval remain separate choices. Local logout removes active credentials, invalidates old admissions and prevents a still-open login from restoring them. It reports local removal separately from upstream revocation, which is not requested or confirmed by this manual flow.

The native manager owns a dedicated MCP private store. Credential leases bind server/profile, exact configuration revision/digest, destination and generation. Local values additionally bind executable hash and executable/working-directory identities; injection goes only to the intended child through the private process control channel. The combined process specification must fit 64 KiB before storage changes. HTTP checks credentials before network work and send, uses a sensitive Bearer header on every request, and never follows redirects or falls back to unauthenticated requests for a credential-bound connection. Discovery, selection, catalog publication and execution recheck freshness. Literal supplied-secret echoes are withheld before catalog/model/artifact publication.

Private account storage does not claim OS-keychain encryption, locked GTK buffers or forensic erasure from SQLite WAL/backups. Literal echo detection does not detect arbitrary transformed exfiltration. Manual token account identity is explicitly unverified. Existing native/provider/GitHub credentials are never fallback credentials for MCP.

## Verification

- **253 selected Rust passes**, including 25 added credential checks. Dedicated MCP frontend tests: **seven passes**; TypeScript and changed-module formatting pass. Platform Clippy retains the same 63 existing warnings.
- Fresh isolated B6 comparison: candidate **243 passes/16 failures** in session tests and **22 passes/six failures** in core tests, identical to baseline including failure assertion bodies. Full frontend candidate **234 passes/two failures**, baseline **232 passes/two failures**, same failure names and primary reasons. Existing assertions are preserved; only new tests and module registrations were added.
- Final Tauri/WebKit/GTK workflows pass for local and remote manual management and private entry, cancellation, save, reviewed connection and logout. Wide/narrow screenshots were inspected. The exact final desktop also passes the credential workflow using a copy of the preserved bound process helper. No inference or capability invocation was used in these UI audits.
- Literal fixture-secret scans cover 43 non-vault files in each of the two final credential runs, with zero matches. Intended private input/peer files and dedicated private vault storage were excluded.
- **673 frozen source files** and **1,275 protected files** were verified unchanged. Old MCP diagnostic `.rlib`/`.rmeta` intermediates were pruned to make build space; every executable, source snapshot and accepted evidence remains preserved.
- The normal production package passed full installer validation, CLI help and cutover usage checks and an isolated native startup/manager/Add server check with zero inference and no visual-test broker. All **1144 installed payload files** and the receipt were verified.

Production desktop SHA: `194b95c790b3e91c8596187d903350dd74b4e012db685f23c48d5259982766a2`. Current user release: `ccbf15608d7c40ddbeb83a9c0e632285`. The prior B6 user release and `/opt/ag1/current` are unchanged; the existing user launcher/menu entry already targets the per-user installation. Existing windows keep their previous binary until restarted.

## Bounds and remaining work

The private vault allows 128 profiles/tombstones in a 1 MiB aggregate envelope, a 48 KiB public header, 16 KiB access/refresh fields, 32 local names with 4 KiB per value, 64 scopes of 256 bytes, and ten-minute pending entries. Quota rejection preserves active credentials. Uncertain storage acknowledgments block current admissions until reopen. Pending operations, configuration CAS, session closure and logout generations fence late results.

Next is OAuth discovery/registration, scoped browser consent, PKCE/state/issuer/resource-bound callbacks, refresh and reauthentication, followed by reviewed trust controls. Public registry discovery/guided setup, supported input continuation, media presentation, delegation scope composition and full A–F compatibility/resource-budget acceptance remain. Other tasks, real-model tests and duration programmes stay paused.

Earlier diagnostic evidence is retained: [vault groundwork](../../MCP/authentication/vault-20260912/progress.json) and [manager/private-entry progress](../../MCP/authentication/manager-20260912/progress.json). These record the corrected fixture/driver failures and earlier candidate identities; later success does not erase those runs.
