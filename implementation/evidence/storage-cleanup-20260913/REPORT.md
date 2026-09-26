# Storage cleanup — 2026-09-13

Completed the authorized AG1 and Steam cleanup.

- Removed 155 obsolete compiler-cache and package directories. No deletion failures.
- Removed 56 old systemwide AG1 releases; retained the current installed release and its immediate rollback release.
- Removed Steam application data, including installed games. Preserved saves, Proton prefixes, settings, game configuration files, and demos at `/home/alex/steam-saves-backup-20260913` (486 MiB).
- Home filesystem available space increased by approximately 355.6 GiB; approximately 355.7 GiB is now available.
- Old system releases accounted for 6.88 GiB; system filesystem now has approximately 9.0 GiB available.

Preserved AG1 source, Git working-tree changes, source archives, evidence, model files, conversations, settings, external llama-server executables, the bound supervisor in product/target/release, the entire product/target/release directory, the .local/p14-target/release build cache, the two newest local package directories, and current/previous installed releases. Symlinked cache directories were excluded. Other projects, Trash, and general system/application caches were outside this cleanup.

Validation: all 969 inventoried source/tool files and five protected runtime/configuration files retained their SHA-256 hashes; Git status was unchanged; the installed CLI version check passed; current release pointer was preserved and two installed releases remain. No application restart, model test, live GitHub action, or rebuild was performed.

Detailed paths and allocated sizes are recorded in plan.json, build-cleanup.json, system-cleanup.json, and steam-cleanup.json. Per-directory allocated totals can double-count hard-linked artifacts; filesystem free-space changes are used for the overall home-space result.
