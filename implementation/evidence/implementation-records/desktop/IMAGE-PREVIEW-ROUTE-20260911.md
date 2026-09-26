# Direct local image preview — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified at `/opt/ag1/releases/27f2f6267ba143dca2a84b6fd2ab7620`.

The failed pelican session attempted an unowned localhost browser URL, then managed preview startup encountered QueueFull. Local images already had a native reader, but its SVG guidance was insufficient and its renderer rejected the blur/shadow filters in the actual pelican SVG.

The model-facing UI tool now accepts `ui({"path":"pelican-bike.svg"})`, routes through the existing workspace-authorized file reader, and returns an image for visual inspection. The description explicitly directs image creation/editing workflows to this path without starting a server or running a conversion command. `inspect` and browser failure recovery also explain the local image route. Existing web URL, interaction and verification routes remain separate request forms; mixed local/browser arguments are rejected. Read classification remains available to CV.

The SVG renderer now supports bounded static blur, shadow and compositing filters (16 filter primitives maximum). Existing input, geometry, output, external-resource and active-content restrictions remain. This is not unrestricted SVG/browser support: unsupported constructs still produce an explicit limitation alongside source text.

## Verification

- 11 tests passed: five surface/schema/classification contracts, four SVG rendering/resource cases, two native image format/resize/original/cancellation contracts.
- The actual `/home/alex/Tests/COD_Z/pelican-bike.svg` was copied into a private workspace and passed through ui routing, native read preparation, SVG-to-PNG rendering and the configured Qwen3.6 vision payload builder. An 800×600 visible image reached the payload. The original file was verified unchanged.
- The rendered pelican image was visually inspected. This verifies render output and provider serialization; no new model inference or aesthetic judgment was tested.
- Production build and archive validation passed. All 1,144 installed files, manifest input hash, archive receipt and protected configuration hashes verified.
- No user desktop capture or user app/model process interruption. Weekly quota snapshot: 2% remaining.

Restart AG1 and start a new chat to receive the updated tool schema and guidance. The installed process is not hot-replaced.

Machine evidence: [image-preview-route-20260911.json](../../P14/image-preview-route-20260911.json). Detailed logs, scoped patch, copied probe source and rendered image are in `.local/image-preview-route-20260911/`. No temporary probe source remains in the product tree. Broader MSP/P14 work remains outside this repair.
