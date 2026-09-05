The terminal escape pattern in ag1-core/src/tools/shell.rs is adapted from
ansi-regex 6.2.2, index.js, from the pinned executable reference. The original
MIT copyright and permission notice are in LICENSE. Rust uses the same OSC/CSI
alternatives with escaped literal brackets and ASCII digit classes. No JavaScript
runtime is required for terminal sanitization.
