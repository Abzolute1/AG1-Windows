# License and provenance audit

This is a repository-grounded reuse inventory for the research stage. No Dyad code/assets have been extracted into a product. The entire clone is an inspection copy, not an approved redistribution bundle.

## Controlling source boundaries

At the pinned revision, root `LICENSE` explicitly assigns Apache-2.0 to content outside `src/pro`, and the separate license inside that directory. `package.json` says MIT; use the actual license boundary, record the metadata discrepancy and preserve provenance rather than relying on the package field. [Pinned Dyad license](https://github.com/dyad-sh/dyad/blob/fb17039bfd2adcac7d74b9c1178a3a3524df35b6/LICENSE).

`src/pro/LICENSE` is FSL-1.1-ALv2: it restricts competing commercial uses and grants a future Apache license two years after a particular software version is made available. No per-version maturity analysis was performed. For this project, VISION's stricter instruction controls: **exclude `src/pro/**` entirely from reuse**, including transitive implementation dependencies and assets. Research inspection is distinct from copying. Do not treat “open source repository” or a future-license clause as clearance for current Pro code. [Pinned Pro terms](https://github.com/dyad-sh/dyad/blob/fb17039bfd2adcac7d74b9c1178a3a3524df35b6/src/pro/LICENSE).

AG1/Cruzible's root `LICENSE` is MIT, copyright 2025 Mario Zechner. Preserve that notice in reused TypeScript reference distributions and derivative code where applicable. AG1 dependencies and bundled vendor libraries still need their own notices; AG1's MIT license does not replace those.

## Candidate clearance matrix

| Material | Inspected evidence | Research disposition / obligation |
|---|---|---|
| Dyad ordinary source/CSS outside Pro | Root LICENSE | Apache candidates: retain applicable notices/license, record modified files and attribution; no trademark permission implied |
| Dyad NOTICE | Root `NOTICE` names Dyad contributors, mustardscript and Playwright | Carry applicable attribution when those portions/dependencies ship; determine relevance from final dependency closure |
| Pro code/assets | `src/pro/LICENSE` | Excluded by project instruction; no translation/copy as an implementation shortcut |
| Components importing Pro | `PreviewIframe.tsx`, `DyadSearchReplace.tsx`, `useAgentTools.ts`; raw `dyad-pro-imports.txt` | Outside-directory files are not automatically prohibited themselves, but cannot be copied with their restricted dependencies. Refactor clean boundaries and use AG1 implementations |
| Type-only Pro imports | `src/ipc/types/agent.ts`, chat stream definition, hook types | Do not carry types into a supposedly clean protocol; design AG1-owned domain types |
| Component tagger packages | Both `packages/@dyad-sh/*component-tagger/LICENSE` files | Apache-2.0 candidates; keep package-specific notices and audit transitive bundle before optional reuse |
| Geist/Geist Mono | CSS font faces and lockfile `geist` license `SIL OPEN FONT LICENSE` | Bundle the exact package font files with OFL text and reserved-name restrictions; verify package contents, not only metadata |
| Lucide | lockfile `lucide-react`; official license page | ISC-based icon distribution includes upstream Feather MIT-derived portions; retain applicable license texts. [Lucide license](https://lucide.dev/license) |
| React/Monaco/Lexical/Base UI/Tailwind/Jotai/TanStack/Virtuoso/xterm | Pinned lockfile license inventory | Mostly permissive package metadata; inspect exact shipped tarballs, notices, workers/fonts and bundled third-party portions before clearing |
| Dyad logos | `assets/logo.svg`, `assets/icon/logo.*` | Replace with AG1 branding; no implied trademark grant |
| Provider/Supabase marks and illustrations | `assets/ai-logos/*`, `assets/supabase/*` | Source presence does not establish all underlying rights. Reference only until original provenance and brand conditions are verified |
| Embedded vendored JS in AG1 | `A:core/export-html/vendor/{highlight.min.js,marked.min.js}` | Inspect distributed headers/licenses and retain notices in HTML export bundle |
| Browser binaries, model binaries/weights | AG1 Playwright/llama runtime; Dyad Electron | Separate redistribution licenses and platform payloads. User-installed runtime versus bundled runtime must be recorded explicitly |

OFL contents for the pinned package have not been downloaded in this task; no font binary was copied. The failed public font-license page lookup is not treated as proof of the full terms. Metadata establishes a review obligation, not final clearance.

## Lockfile findings

`evidence/dyad-dependency-license-inventory.json` records all 1,783 package entries, including root, dev tools, links and platform variants. Counts are package records, not unique dependencies or shipped artifacts. Recorded labels include MIT 1,431; ISC 134; Apache-2.0 105; BSD variants; MPL-2.0 11; LGPL-3.0-or-later 9, plus four mixed Apache/LGPL labels; missing license metadata 10. Many entries are dev-only/platform-specific and may never ship.

Pay particular attention to `@img/sharp-libvips-*` and Sharp WASM/Windows bundles carrying LGPL terms; `caniuse-lite` (CC-BY-4.0), `spdx-exceptions` (CC-BY-3.0), `geist` (OFL), and packages with `SEE LICENSE IN LICENSE`. Missing metadata includes local linked packages and `@dyad-sh/supabase-management-js`, `@vercel/sdk`, `browser-assert`, `buildcheck`, `cpu-features`, `ssh2`. Missing metadata is **unknown**, not unlicensed or forbidden; inspect each actual distribution if retained. Most Electron/cloud tooling can be excluded from the target dependency set altogether.

No full binary SBOM, transitive import closure, copied-design-resource history audit, or redistribution-law assessment was completed. These are explicit implementation/release gates, not reasons to inherit the original dependency tree. Do not describe the whole Dyad checkout as Apache-only or MIT-only.

## Required extraction ledger and gates

Before each copy, create a manifest row with upstream URL, pinned commit, original path, file SHA-256, governing license, original copyright, destination, changes and applicable third-party notices. Mark each row approved candidate / excluded / unresolved. For CSS and assets, record the same information as for code.

Resolve and inspect **transitive** imports (including type imports, barrels, CSS URLs, worker bundles, dynamic imports and public assets). The 68-line Pro import search is only a starting point. Fail extraction checks on `src/pro`, Dyad production API/telemetry keys, unresolved licensed assets or prohibited dependencies. Review a dependency allowlist and actual production bundle; a text grep alone is insufficient.

Before redistribution: generate the target's own dependency/license inventory and binary SBOM; include applicable Apache LICENSE/NOTICE, MIT/ISC/OFL and other third-party texts; add modified-source notices; verify bundled browser/model/native binaries; replace Dyad marks; review unresolved records. Keep the reference clone outside release/package inputs. A successful TypeScript build is not a provenance gate.
