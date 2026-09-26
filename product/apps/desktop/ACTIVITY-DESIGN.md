# Iterating on AG1's activity indicator

Run the desktop Vite development server with the repository's Node 24 toolchain, then open `/activity-preview.html` (default `http://127.0.0.1:1420/activity-preview.html`). The studio uses simulated activity and the real UI components; it needs no native bridge, model or GPU. It is a separate development entry and is not included in the production app entry.

The studio has dark/light themes, a motion-cycle slider, Pause/Resume, Next verbs and a conversation-phase selector. Its conversation example exercises the actual `TurnView`, including the live status below streamed output. Use browser reduced-motion emulation to inspect the fully static version.

## Small, independent editing surfaces

- `src/ActivityMark.tsx`: SVG geometry only. The initial mark has three short rounded strokes at 0°, 120° and 240° around an open center. Change the path, angles or viewBox here.
- `src/activityMotion.css`: animation, theme colors, sizing and accessibility fallbacks. Shared cycle coordinates the travelling highlight with the word shimmer. No frame timer, state update or native call drives the motion.
- `src/workingVerbs.ts`: 254 phrase entries grouped by actual activity. Add or remove phrases without touching components or the native protocol. Selection is deterministic from turn/operation/category identity, not elapsed time or streamed tokens.
- `src/ActivityStatus.tsx`: native display-state mapping, conservative tool classification and accessible literal status. Keep this independent from visual experiments.
- `src/ActivityStatus.preview.tsx`, `src/activityPreview.css`, `activity-preview.html`: development studio only.

CSS variables can be overridden on the status or any ancestor:

| Variable | Initial value | Purpose |
|---|---|---|
| `--ag1-activity-cycle` | `2400ms` | Shared stroke/shimmer cycle |
| `--ag1-activity-size` | `20px` | Fixed mark footprint |
| `--ag1-activity-stroke` | `2.2px` | Line thickness |
| `--ag1-activity-travel` | `-.7px` | Radial stroke movement |
| `--ag1-activity-gap` | `9px` | Mark-to-label spacing |

Colors use the existing `--muted-foreground` and `--foreground` theme values. There is no green/success accent. Reduced motion and forced colors disable both animations and leave readable text plus a static mark. Paused/approval/error states do not pulse.

## Preserve truthful status

Whimsical phrases belong to real thinking, output generation, request preparation or unresolved tool execution. Do not infer an action from the user's prompt, a completed historical tool call or text inside `echo`. Shell classification is intentionally conservative; unfamiliar commands get a generic executing phrase.

Keep loading, verification, queueing, awaiting-first-output, stopping, approvals and errors literal. `Waiting for model` alone does not prove prompt processing and must not be labelled as measured reading progress. The shimmer conveys activity, never completion percentage, successful checks or elapsed-time progress. The top response timer and completed tool labels retain their existing wording.

Run `npm test -- --run src/ActivityStatus.test.tsx src/NativeConversation.test.tsx` plus the TypeScript/Vite build after behavior changes. For animation-only changes, inspect dark/light themes, a narrow viewport, reduced motion and frozen animation frames; ensure the mark footprint and text baseline do not move. Final implementation evidence is linked from `implementation/README.md` at repository root.
