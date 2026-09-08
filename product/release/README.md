# AG1 local release candidate

This Linux x86_64 candidate contains Tauri/React/Rust, the CLI, cutover inspector,
process supervisor, notices and a dependency inventory. It is unsigned and is not
release acceptance. See the checkout's `implementation/evidence/P14/completion.json`.

Build from the pinned checkout with `python3 tools/release_package.py` after
`npm --prefix product/apps/desktop run build`. Cargo uses `.local/p14-target`.
The active supervisor in `product/target/release` is never a build output here.
The final P14 gate validates the exact existing candidate when its source digest
still matches. It verifies a fresh isolated installation and retained build logs;
it does not substitute another build for the bytes used in duration measurements.

Inspect the package SHA-256 and manifest before local installation:

```sh
python3 product/release/install.py ARCHIVE REVIEWED_SHA256 NEW_INSTALL_DIRECTORY
NEW_INSTALL_DIRECTORY/current/run-ag1
NEW_INSTALL_DIRECTORY/current/bin/ag1-cli --help
```

The installer only writes a versioned payload, receipt and atomic `current` pointer
inside its installation directory. Updates retain previous versions. An interrupted
operation leaves `pending.json` and its stage/receipt; inspect it before any retry.
It never performs agent-state migration, runtime configuration adoption or rollback.
The Python installer is a local review tool; Python is not required to run AG1.
It requires Linux/POSIX. Other platform installers remain pending.

The launcher stores desktop preferences in `$XDG_STATE_HOME/ag1/desktop` (or
`$HOME/.local/state/ag1/desktop`), separately from installation files. Durable
workspace authority still uses the OS account registry. For an already migrated
workspace it resolves the accepted native destination and configuration. Opening a
legacy workspace requires its separately retained TS reference companion; the
default candidate does not include that entire AgentSession or Node. Keep the
checkout and P13 archives available. Missing legacy dependencies are not permission
to migrate or reopen stale original journals.

`external-runtimes.json` records the existing three tuned Linux model executables,
profile and plan hashes. Libraries, weights, projectors and credentials remain
external. Missing or mismatched identity blocks runtime admission. Installation
does not replace the bound process supervisor in existing native configuration.
Adopting a candidate supervisor requires a fresh quiescent configuration transition.
No stock llama.cpp fallback, tuning, profile rewrite or model reinstall is performed.

The extension archive is separate and requires an explicitly configured existing
Node runtime; browser automation and project toolchains are external optional
dependencies. Retain the compatibility matrix, including unsupported native
embedding, judge-only vision and dynamic extension-provider rebinding.

`sbom.cdx.json` inventories renderer packages and conservatively includes Cargo
build and other-platform license entries, plus the compiler's Rust standard-library
notices and third-party exceptions. `binary-dependencies.json` records actual
Linux loader closure; these system libraries are external, not redistributed.
This is not a portable Linux ABI certification or an exact static symbol-level SBOM.

All L01–L08 preservation requirements remain in force: per-model binary/library
identity, exact profile precedence, model/request identity, no implicit tuning,
exclusive generation ownership, authenticated process control, retained originals
and recovery, and matched TS/Rust evaluation conditions. Keep the reference and all
P13 checkpoints until full P14 acceptance. Publishing requires separate instruction.
