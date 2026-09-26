# MCP Discover larger type and icons — 2026-09-13

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Complete and installed systemwide. Both launchers use the new build; restart AG1.

Incremental sizing adjustment requested after the row redesign: icons grow from 40 to 48 px, server names from 16 to 18 px, descriptions from 14 to 16 px, metadata from 13 to 14 px. Minimum row height grows from 108 to 120 px. Selected details use 16 px body text, 20 px names and 14 px facts. The 62/38 split, native row styling and existing actions remain intact.

Only Discover CSS and image dimensions changed. The existing visual check passed at 640, 1,000, 1,440 and 2,560 px, without horizontal overflow. Before-edit files, source hashes and visual evidence are retained under `.local/mcp-scale-ui`. No new tests or repeated backend suite were needed for this sizing change. A fresh production profile and normal package validation preserve earlier releases.

Production typecheck/build, package validation and native screenshot review passed. Installed release: `/opt/ag1/releases/17e9bb4d4186436889cbf6254088c141`. Desktop SHA-256: `f03a571bdac332655248bbcec7eb160368f3dda3eb5006c8c2fc1d3a75fb1d53`. Archive SHA-256: `c3607d7cdef2c28be115be6f1f20e62ea91ec808eac982ba82fe0df447f4305d`. Earlier releases are preserved. Installation receipt and screenshot review are recorded in `.local/mcp-scale-ui/installation.json` and `production-inspection.json`.
