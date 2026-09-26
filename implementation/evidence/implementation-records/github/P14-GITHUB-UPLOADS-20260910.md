# GitHub release upload checkpoint — installed

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Continued GitHub-only work and systemwide installation after every major verified checkpoint are explicitly authorized. At each installation, give the user a brief description of what remains and the expected amount of work. GitHub remains incomplete; finish its matrix, report, and stop before any other phase.

## Implemented

`releases.upload_asset` uses the shared native UI/model approval service. A source must be an exact regular file under a current granted folder. The native preview retains up to 64 MiB of exact bytes and binds the file revision, SHA-256, folder-grant revision, account/credential generation, conversation, destination release, asset name, media type and expiry. At most two upload plans retain bytes concurrently. Changed/replaced/symlinked sources, revoked or changed grants, stale accounts/scopes, cancellation and consumed approvals are refused. Upload bytes never become ordinary operation arguments or preview JSON. Generic binary write APIs remain blocked.

The release browser has an upload form, explicit publication acknowledgement, readable destination filename and exact source metadata. UI and model use the same prepared native operation. Fixed `uploads.github.com` HTTPS transport revalidates the account and source before sending raw bytes; no redirects or retries. Only a matching 201 receipt with positive asset ID, exact name/size, uploaded state and matching digest when supplied confirms success. Errors after send are uncertain; the app does not automatically delete or replace existing/starter assets. Names are conservatively limited to ASCII alphanumeric, dots, underscores and hyphens; no arbitrary upload URL. The [official release assets API](https://docs.github.com/en/rest/releases/assets?apiVersion=2026-03-10) was checked for upload framing and failure semantics.

## Verification

- Typecheck and **209 frontend tests / 26 files** pass; the final filename presentation change passes the targeted four upload tests and typecheck again.
- **57 GitHub unit tests** pass after the final cancellation/source checks. Owned real TLS covers exact non-text body and headers, redirect refusal, permission denial, interrupted receipt and duplicate-name rejection without forwarding or retry. Source tests cover changed files before/after approval, grant/account/scope changes, cancellation, uncertain outcomes, quota, oversize and symlink refusal.
- Final native candidate `native-candidate-l01eXw/upload-ruLB9u/result.json` passes: no upload before approval; changed source rejected and later bytes preserved; fresh UI upload, refreshed asset metadata and exact binary download; model upload with native approval. Two owned uploads, one prompt/two scripted provider requests. Narrow upload preview screenshot inspected.
- Earlier candidate `oGv2Pp/upload-idaMRP` passes the same workflow, and `download-QoEBD1` plus baseline `github-1Sg66Y` pass affected regressions. The final candidate adds only the prominently displayed filename and cancellation error preservation. No live account/repository writes or inference.
- Production renderer, isolated offline release build, bundle/license/native library checks and isolated production startup pass. Startup has zero provider requests and no leaked owned children; screenshot inspected.

Evidence: `.local/p14-github-resume-20260910/upload-*`. No frozen comparison expectations were changed. The final full matrix/regression run remains outstanding.

## Installed identity

Package `.local/p14-packages/3cc391b85afd4fafb2bc9d2f0a0fc11d/ag1-0.1.0-rc-linux-x86_64.tar.gz`; archive SHA `36da118062ace89aa91455288ef59a56da97606761f8151d8af0513420cd0ec1`; source digest `261384bf1db457509fe56fb71234528ac913036e44898f209c4c7221820ef141`.

Installed at `/opt/ag1/releases/2d0946b7fdaf476185cde6cded62fa61`; desktop SHA `eef6ebdeed29362386e14024a80bd5d1c5b399fae52a0d31fe160217da04973d`. All 1,140 payload files match bytes, size and mode. Receipt/archive match; no pending installer state. Previous download release retained. Four protected runtime/helper/configuration hashes remain unchanged. Exact report: `upload-installation-verified.json` in the evidence directory. Private stdin installation authentication created no password file or persistent memory.

Remaining substantial areas: private secret entry; fuller Actions/releases/inbox workflows; writable linked-worktree metadata authority and aggregate repository/journal quotas; final capability/authority/regression verification. This is still hours of work, with local Git authority and failure recovery carrying the largest uncertainty. Next: private secret entry.
