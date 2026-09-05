The preview-only parser in ag1-platform/src/extension_partial_json.rs is a bounded
Rust adaptation of partial-json 0.1.7 (MIT, copyright Muspi Merol), as used by the
pinned AG1 parseStreamingJson helper. The license is reproduced in LICENSE.
It is used only for optional extension stream projections. Final tool arguments
continue through strict parsing and native validation. No JavaScript parser is
bundled by this port. The port bounds depth, node count and bytes, rejects non-finite
values at the typed JSON boundary, and preserves UTF-8 boundaries on malformed input.

Pinned dependency source hashes (SHA-256):
- `dist/index.js`: `23f750aa1170c830b7ef180135852317886cc58ce7d2597e5eeee044539072c7`
- `dist/options.js`: `779b2aded6d4e2ec6171ea9d5d5d97551e8c3258b6e78a01d2e9fc6235865b36`
- `package.json`: `42b3ae6c99ec959249f8eb4d3da7fdb44ec1f96ec627fa7f9ca5487a4e80963f`
- `LICENSE`: `cd519ad3d7e012427f978dfb2e3b92ee403d189d7b859ba6bf68fd7e12ca456f`
