# GitHub download checkpoint — installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The user authorizes continued GitHub-only implementation and systemwide installation after every major verified checkpoint. Full GitHub remains incomplete; stop and report only after its entire acceptance matrix is complete. Do not begin MCP or unrelated work.

## Implemented

Shared native `actions.download_artifact`, `actions.download_logs`, `actions.download_job_log`, and `releases.download_asset` use fixed GitHub API paths. The native transport handles direct bytes and bounded redirects to GitHub download hosts; redirected requests have no account Authorization, cookies, referer or API headers. Signed locations remain native. No arbitrary origin, automatic retry, archive extraction or execution is admitted. Exact credential echoes are discarded rather than exported as a file.

Downloads use ephemeral native account/credential-generation/conversation-bound receipts. Ordinary replies contain only receipt metadata. `downloads.read` returns bounded base64 byte pages after native account and scope checks. Maximum 64 MiB per file, 128 MiB retained per service, 32 receipts, 256 KiB per page. The store reserves room for a full-size incoming file before transfer. Access expires after 15 minutes; expired bytes are reclaimed on the next store request or service drop/disconnect. No persistent download archive is created automatically.

Settings browser, workbench and model result cards share `GitHubDownload.tsx`. Saving is an explicit user action and checks the full SHA-256 before creating an inert Blob download. Byte pages, identity/ranges, late scope replies, cancellation and disconnect are checked. Job log preview shows at most 65,536 bytes as literal UTF-8; malformed characters are replaced. ZIPs are not unpacked. Generated names retain the operation and numeric target; release downloads currently use `.bin`, rather than claiming original metadata filenames or server digest attestation.

A native audit caught desktop API scope using the client bridge epoch while model tools used the runtime context epoch. The desktop now obtains `Host::github_scope()`, shared with local Git, so model download receipts can be opened from their native result cards while staying conversation-bound.

Primary references checked: [Actions artifacts](https://docs.github.com/en/rest/actions/artifacts?apiVersion=2026-03-10#download-an-artifact), [workflow logs](https://docs.github.com/en/rest/actions/workflow-runs?apiVersion=2026-03-10#download-workflow-run-logs), [release assets](https://docs.github.com/en/rest/releases/assets?apiVersion=2026-03-10#get-a-release-asset). GitHub.com remains the supported host; Enterprise is unsupported.

## Verification and actual evidence

- Typecheck; **205 frontend tests / 25 files**; **54 GitHub unit tests**; three native client Git/folder/review contracts pass.
- Owned actual HTTPS fixture proves credential-free redirect requests, exact non-text bytes, advertised and chunk-streamed 64 MiB over-limit refusal, interrupted response, permission/rate failure, credential-echo discard, pre-send cancellation and cancellation during a hanging transfer. Native storage tests cover exact paging, stale accounts/credentials/scopes, disconnect/reconnect, one-use model confirmations, expiry and aggregate quotas.
- Candidate `native-candidate-vfFnwe/download-bWWG4F/result.json` passes four UI download types and model approval/receipt/save. Each archive/asset saves exactly 300,000 non-text fixture bytes, and job text preserves exact CRLF bytes. One prompt/two scripted provider requests, no API writes or live traffic. Narrow log screenshot inspected.
- Same candidate passes shared baseline `github-dAwQAl`, graph actions `graph-edit-VpxeVv`, and native recovery/staging `github-recovery-hoRlRX`. These are owned fixtures, not live GitHub/model evidence.
- Production build, renderer audit, licenses and native library checks pass; isolated production shell startup passes with no prompt/provider traffic or leaked owned children.
- Earlier failed native runs are retained: `8DxtO1/download-qyDmhc` had a harness Load/Refresh label mismatch; `8DxtO1/download-S7kOAM` exposed the real model/UI scope mismatch. Both exact workflows passed after their fixes. Initial Rust character/string replacement and typecheck fixture types were corrected; full reruns passed. No frozen expectations were changed.

Logs: `.local/p14-github-resume-20260910/download-*`.

## Installed production identity

Package `.local/p14-packages/e288def187d746a8bea22f767fe79d8b/ag1-0.1.0-rc-linux-x86_64.tar.gz`, archive SHA `08ac555c144b30c0ace125ff4ad058e71c39ce7a3eb99606b63ce7ae71aa3c29`, source digest `3fae3af1e41cf00554227c78a758bdb22cc32b4863d3953704bf26b47771f170`.

Installed at `/opt/ag1/releases/300816d67f294c78b784377b56b6c3af`; desktop SHA `8019f2c6b1c67351942fb66479edd0fb98ebeaedce7122a647c1ce1fe40ae7d0`. All 1,140 payload files match bytes, size and mode; receipt matches the archive; no pending installer state. Previous `b0b1490b737c459ea008ad99e5e39449` release retained. Protected release helper, runtime profiles, runtime capture and native-host configuration hashes remain unchanged. Exact verification: `download-installation-verified.json` in the evidence directory.

The authorized installation used private stdin authentication; no password was stored in source, logs, project files or persistent memory. Subsequent development is not part of this sealed checkpoint.

Next: release asset upload custody and forms; private secret entry; richer Actions/releases/inbox; writable linked metadata and repository/journal quotas; final full capability/authority verification. Download support does not complete GitHub.
