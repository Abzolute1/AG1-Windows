# MCP Discover readability adjustment — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Complete and installed systemwide. Both launchers use the new build; restart AG1.

A small additional sizing pass: server icons 48 → 52 px, names 18 → 19 px, descriptions 16 → 17 px, metadata 14 → 15 px. Row vertical padding increases from 16 to 18 px (14 to 16 px on narrow screens), with a 128 px minimum height. Horizontal rows, separators, adjacent Add actions and the existing master/detail split remain intact.

Only Discover CSS and image dimensions changed. The existing visual check passed at 640, 1,000, 1,440 and 2,560 px without horizontal overflow. Evidence and before-edit files are in `.local/mcp-readable-ui`. The normal production packaging process reuses the previous Cargo build cache to avoid rebuilding dependencies and exhausting disk space; installed and archived releases remain unchanged.

Production typecheck/build, normal package validation and native screenshot review passed. Installed release: `/opt/ag1/releases/e187399b84ac4a0aa2b4e937e808451f`. Desktop SHA-256: `5ba5bac7503fea32f10513dba1fb1a63ad6de6a81d263768b575e7dd91b7f194`. Archive SHA-256: `a976bb3544f1b1f1b0bd733496c20bbd149be4b049f8c84c8bc4482f6880f9bb`. Installation and review evidence are recorded in `.local/mcp-readable-ui/installation.json` and `production-inspection.json`. Prior installed releases remain available.
