# Project folder picker — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified: `/opt/ag1/releases/0e2c2ab0b01e4c0586daa466d40f6db2`.

The sidebar Projects + button now opens the existing native folder picker immediately. The project dialog also has a Browse folders button, retains direct path entry and recent projects, and preserves the selected path on picker cancellation. The shared GTK folder picker explicitly enables folder creation and uses the neutral title Choose a workspace folder. Select/create a folder, choose it, then click Open project to use it as the workspace. No new dependencies.

32 App tests passed with the pinned Node 24.14.0 toolchain, including picker invocation, selected path and cancellation. Frontend typecheck/build, native production build and archive validation passed. All 1,144 installed files, receipt and protected configuration hashes verified. No user desktop capture or real native picker interaction was performed; dialog behavior is covered by the existing GTK API integration and frontend mocked bridge test. Initial tests under system Node 26 failed because of its experimental localStorage global; rerunning under the established pinned toolchain passed.

Restart AG1 to load the update. Detailed patch, test/build logs and installation receipts: `.local/project-folder-picker-20260911/`. Weekly usage snapshot now reports 0% remaining. No other pending task was started.

[Machine evidence](../../P14/project-folder-picker-20260911.json).
