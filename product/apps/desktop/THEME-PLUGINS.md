# AG1 desktop theme plugins

Theme plugins are data-only `.ag1-theme.json` files. In Settings → Appearance, choose **Import** to install a downloaded file. The dropdown lists the bundled and installed themes. Selecting a theme clears the three custom color overrides so its palette is visible. Other typography and layout preferences remain yours.

**Copy theme** copies an editable JSON definition. **Export theme** writes a complete file, including your color overrides, into the desktop state's `theme-exports` directory and shows the resulting path. Existing exports are never overwritten. **Installed theme plugins** provides removal; removing the active plugin returns to AG1 Graphite. Importing a duplicate ID is rejected rather than silently replacing it.

Start with an exported theme or `src/builtin-themes.json`. Each file contains one object:

```json
{
  "schemaVersion": 1,
  "id": "your-theme-name",
  "name": "Your Theme",
  "mode": "dark",
  "colors": {
    "background": "#1a1a1e",
    "foreground": "#efeff1"
  }
}
```

The abbreviated example needs the complete palette before it can be imported. Required keys: `background`, `foreground`, `sidebar`, `card`, `popover`, `muted`, `muted-foreground`, `border`, `primary`, `primary-foreground`, `accent`, `accent-foreground`, `input`, `ring`, `secondary`, `secondary-foreground`, `destructive`, `code`, `link`. Every value must be a six-digit hex color. Optional keys reserved for syntax colors are `syntax-keyword`, `syntax-string`, `syntax-comment`.

IDs contain lowercase ASCII letters, numbers and hyphens, up to 80 characters. Use a unique ID; bundled IDs and `system` are reserved. `name` is a nonempty display name up to 80 UTF-8 bytes. `mode` is `dark` or `light`. The native host enforces version, size (32 KB), palette keys, field types and a 64-plugin library limit. Unknown top-level fields, CSS, scripts, URLs and executable hooks are rejected. Theme plugins never run package installers.

Installed theme definitions and selection are kept in the native desktop catalog. Existing `dark`, `light`, `tokyo` and `system` preferences continue to work. `dark` now displays as **AG1 Graphite**. Bundled palettes are data rather than separate UI cards; adding one palette does not require another picker component.

This is the desktop palette format. Existing terminal resource themes remain separate; a terminal theme file must be converted to the complete desktop palette before importing. A marketplace or remote theme downloader is not included.
