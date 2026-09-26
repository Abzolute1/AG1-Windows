# Combined chat presentation cleanup

Status: planned combined pass. Existing activity summaries, thought controls and change/output/GitHub cards must be preserved and reused. Read the [task index and shared rules](README.md); do not begin implementation solely because this file exists.

## Completed focused work

The user explicitly resumed thinking placement and website-icon presentation on 2026-09-13. [Implementation and verification report](../implementation/evidence/chat-activity-20260913/REPORT.md): live status/thinking follows work and result cards, completed reasoning is preserved as Thoughts, and native web-result source badges/links are implemented. 561 final automated checks passed; two bounded private native 35B runs inspected real icons and layout. Production package is installed systemwide; all 1,144 installed payload files and the receipt were verified. Both live search queries returned no results; direct page fetching worked. This focused work does not close the combined checklist below.

## New findings from bounded comparison

The [35B / OpenCode comparison](../implementation/evidence/bounded-comparison-20260913/REPORT.md) observed working favicons and correct live-status order in 920 sampled frames. Follow-up work: webFetch errors can still appear as completed reads with source badges, and official fallback suggestions are not distinguished from search hits. Research quality also needs fuller/relevant source extraction. These are findings, not delivered fixes or authorization to resume the broader endurance programme.

The focused [DuckDuckGo repair](../implementation/evidence/duckduckgo-fix-20260914/REPORT.md) was installed on 2026-09-14: blocked searches now fail clearly, static suggestions are separated from hits, and source badges suppress suggestions/explicit fetch errors. 302 automated checks passed. DuckDuckGo still blocked the live request; better live search quality is unproven. Full webFetch error-state propagation and richer documentation extraction remain open.

The subsequent [DDG request-flow investigation](../implementation/evidence/duckduckgo-request-flow-20260914/REPORT.md) installed POST transport and serialized public searches without a fixed success delay. Three of seven native attempts returned real results, but challenges recurred even after accepted browser verification. No verified numeric rate limit was found. Twenty-two focused checks passed; reliable live DDG access remains open.

The [free-search recovery](../implementation/evidence/free-search-recovery-20260914/REPORT.md) subsequently installed DDG's no-AI web result flow with bounded arithmetic handling. Fifteen native queries completed without blocking; 13 returned results and two controls were correctly empty, with median 807 ms. Thirty-nine focused checks passed. This resolves the observed live-search failure for the tested checkpoint; future DDG changes and the separate webFetch/combined presentation work remain open.

The [DuckDuckGo engine overview](../implementation/evidence/search-engines/DUCKDUCKGO.md) explains the installed search flow, its effect on results and proposed quality improvements. The user subsequently resumed its improvements. The [search-quality checkpoint](../implementation/evidence/search-quality-20260914/REPORT.md) installed candidate ranking/cache reuse, richer DOM/source excerpts and explicit fetch failure outcomes. Fifty-four focused checks passed; native search and evidence delivery improved. The bounded 35B still made reasoning errors and exposed a separate CLI research/coding-completion classification issue. These findings remain open; the combined presentation checklist below is not closed.

## Intended result

The [AG1 activity mark and verb library](../implementation/evidence/activity-personality-20260914/REPORT.md) are installed: themed shimmer, custom three-stroke mark, 251 distinct phrases, stable operation-based selection and a bottom status that remains visible during streaming. Literal loading/waiting/approval states and reduced motion are preserved. A separate design studio supports further animation iteration; this does not close the combined checklist.

Focused website-icon reliability work is [completed and installed](../implementation/evidence/website-icons-20260914/REPORT.md): declared/CDN icons, static SVG, queued fetching, retries and domain fallback. The OpenAI logo failure was reproduced and recovered. Broader combined presentation work below remains open.

Everyday results should be readable directly in the conversation or through one useful review/details entry point. Failures and pending decisions stay easy to find. Exact arguments, raw output and diagnostic records remain available when needed.

## Remaining tasks

- [ ] Inspect the current conversation presentation after artifact/review and MCP work. Identify actual repeated nesting and unnecessary controls; do not replace existing working summaries without a demonstrated problem.
- [ ] Consolidate successful work into concise activity summaries and useful output/change cards. Reduce repeated Activity → tool → Arguments → Output nesting for normal results.
- [ ] Keep failures, uncertain effects and pending approvals immediately discoverable. Preserve exact technical arguments/raw logs on demand and avoid falsely presenting failed or merely accepted operations as completed work.
- [ ] Preserve thought visibility controls, streaming/call pairing, copy fidelity and long-history behavior. Bound rendering and avoid repeated large-content hydration. Existing containment retains loaded DOM/React state; it is not DOM virtualization.
- [ ] Preserve accessibility, keyboard/focus behavior, chat position and drafts as cards and panels open, close or update.
- [ ] Review empty/loading/error/cancelled states and terminology across artifacts, local changes, GitHub, MCP and approvals. The visible result must agree with the native outcome and the assistant's action.
- [ ] Check wide/narrow layouts, long names/logs, multiple concurrent/pending activities and mixed results using focused frontend and native fixtures.

## Acceptance

Users can understand the main result and any required decision without opening debug records. Failure details and exact source remain accessible. Existing streaming, history, thought, approval, artifact, GitHub and MCP behavior survives the presentation changes.

Capture a current native conversation example with screenshots and inspect it for unnecessary nested controls. Run appropriate regressions, build/install at a major verified checkpoint, and report what remains for [combined final verification](04-FINAL-VERIFICATION.md).
