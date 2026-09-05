# Frontend provenance inputs

`dyad-extraction-ledger.json` records 37 candidate roots, source hashes, planned destinations and conservative transitive dependency discovery. No root is yet approved as a complete product bundle. Restricted closures must be decoupled; unresolved imports/assets need compiler/manual review in P04. No Dyad source has been copied into a product.

`package-evidence.json` records exact lockfile tarball URL/integrity, captured license texts and font-file hashes for Geist, Lucide, React, Base UI, CVA, clsx and tailwind-merge. Tarballs are inspection-only under ignored `reference/third-party/`. Geist 1.5.1 includes 40 font files; its included license controls later use. Applicable license/notice texts are retained here. No model/browser redistribution clearance is implied.

Visual baseline scenarios for P04: home/recent projects, streaming chat and 1,000-message scroll, dirty file/conflict, diff, preview loading/error, model/settings dialog, approval, toast, collapsed/resized panes, light/dark, reduced motion, keyboard focus and font scale. Capture pinned Dyad and AG1 at identical viewport/scale with actual packaged fonts/Monaco workers. Source inspection cannot pass the visual gate.

The pinned Dyad checkout requires Node >=24 <26. Provision a separate supported Node 24 environment, install with its lockfile in an executable inspection copy, build the package with platform-native dependencies, and use a real desktop or configured display. Keep the vendor checkout unchanged. Record actual build/launch commands, OS/webview/GPU and package hashes; do not compare checkout directory sizes or mix model services into only one side's measurements.
