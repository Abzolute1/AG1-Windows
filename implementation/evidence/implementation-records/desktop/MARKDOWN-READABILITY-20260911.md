# Markdown readability — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Status: **installed and verified**, release `/opt/ag1/releases/dd94f46abadc4e9cae7e7a22d5f5d99a`. All 1,144 payload files, installation receipt and protected hashes verified. Previous release `1d7bca72efe74a778881314d15f3c77f` remains retained. Weekly usage snapshot: 4% remaining.

The shared desktop Markdown renderer now groups list items more tightly, strengthens inline-code backgrounds, and uses consistent heading spacing. Code fences have a language label, syntax highlighting and an independent copy button with success/error feedback. Copying preserves the rendered code text, including indentation and trailing newline, without copying the toolbar.

Highlighting uses pinned `lowlight` 3.3.0 and its common grammars. Explicit language fences are preferred; short unlabelled blocks use bounded automatic detection. Explicit plain text, unsupported languages, and blocks over 32,768 characters remain literal and copyable. Automatic detection is limited to 4,096 characters and seven common languages. Generated tokens are rendered as React text/spans, never executable HTML. [Dependency API](https://github.com/wooorm/lowlight#api).

The change is confined to the shared renderer, a scoped stylesheet, its tests and frontend dependencies. Existing chat size, line-height, code-font and wrapping preferences remain effective. Model prompts, tools, permissions, CV budgets and runtime settings are unchanged. Existing messages also benefit when rendered after restarting AG1; a new chat is not necessary.

Validation: all 15 focused Markdown, conversation, tool-output and preference tests passed. Frontend TypeScript/Vite build passed. An isolated headless browser rendered a sample matching the user's comparison in dark and light themes at 1,100 pixels and midnight at 430 pixels. All three had no page overflow or runtime errors, visible highlighting, and exact clipboard content. Screenshots were visually reviewed. This captured only the owned browser fixture, never the user's desktop. Production package/install evidence is recorded in the machine summary.

Evidence: `.local/markdown-readability-20260911/`, including saved original files, scoped patch, build/test logs, screenshots and clipboard/layout checks. [Machine summary](../../P14/markdown-readability-20260911.json).
