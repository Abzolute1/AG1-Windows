# Desktop UI follow-up delivery — 2026-09-09

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Implemented in the settings-delivery pass; installation and validation identity live in `evidence/P14/desktop-settings-delivery.json` after packaging.

- Approval choices retain their policies and use Hand, List checks and Lightning icons. The composer reflects the selected policy.
- Appearance is inside Settings; the standalone sidebar theme toggle is removed.
- Settings uses grouped left navigation, searchable functional sections and separate scrolling. Archived chats and Trash are distinct.
- Theme cards are replaced by a dropdown, palette preview and editable colors. Built-ins and imported plugins share one data format. Import, copy, export and remove are implemented. Exports are saved under the desktop state's `theme-exports` folder, with the exact path shown.
- Discord Dark is renamed AG1 Graphite. Tokyo and AG1 Paper remain available. AG1 Midnight uses the supplied GitHub screenshot's palette without that branding.
- Chat menus include Delete chat, which explicitly moves the conversation to Trash. Settings can restore it. This does not erase journals or shared artifacts; discovery preserves the deletion and renamed title.
- The model picker opens on the selected model and its reasoning controls. Toggle-only and effort-level profiles render differently. Native validation repeats capability checks before applying an explicit selection. Unsupported or unknown capabilities produce an explanatory state, not invented levels.
- Reasoning capability sources are the exact loaded llama server's model identity and template, or explicit metadata from a provider integration. No model-name guess supplies the UI menu. Cold/unavailable/authenticated llama endpoints without a declared profile may need a refresh after loading. API providers without enumerated capability metadata remain unavailable for this control; a boolean is insufficient.
- Native effort mappings survive resume and preserve native enum spellings; the off/none mapping and xhigh level are covered by request tests. Existing launch profiles and sampling tunes are unchanged.
- Plugins and skills use an installed library, search/category filters and separate Add plugin, Create skill and Advanced views. Installation capabilities and approvals remain backed by the host.
- Plan mode adds a faint teal composer outline. Its color is independently editable in Appearance. The text chip remains visible.
- Private Xvfb/Tauri/WebKit checks cover these changes, including import/export, reasoning reaching a streaming fixture, Trash/restore, and narrow windows. No user-display control or extra real-model inference is needed. Neither long duration run is started.

See `product/apps/desktop/THEME-PLUGINS.md` for the theme plugin contract. A public marketplace and automatic metadata integration for every remote provider are not claimed by this pass.
