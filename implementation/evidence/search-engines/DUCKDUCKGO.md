# DuckDuckGo — AG1 search implementation

Updated: 14 September 2026. This is the readable engine overview. Detailed dated reports and raw responses remain in their original evidence folders. The latest installed quality improvements are summarized below; earlier reports remain historical evidence.

## Engine and current status

| Item | Current implementation |
|---|---|
| Search engine | DuckDuckGo |
| AG1 result-provider identifier | `duckduckgo-web` |
| Search entry point | `https://noai.duckduckgo.com/` |
| Result-data endpoint | `https://links.duckduckgo.com/d.js` |
| Credentials/payment | No account, API key or paid service used by this flow |
| Requests per uncached search | Normally two; three when the supported arithmetic check occurs |
| Fixed pause between successful searches | None; requests are serialized and results are cached |
| Verified installed checkpoint | `/opt/ag1/releases/7dbee4adb4ba4f1386b96a1a96809573` |

The [latest installation and quality report](../search-quality-20260914/REPORT.md) identifies the exact build. Resolve `/opt/ag1/current` before assuming that it remains the installed version.

## What changed and why

The original HTML/lite search path frequently received a human-verification page. AG1 initially treated those pages as successful searches with zero results. Earlier repairs corrected parsing, explicit-query preservation, error reporting and separation of static suggestions from actual search hits. POST form submission recovered some results but did not make that route reliable on this connection.

The working change follows DuckDuckGo's separate no-AI web-search flow:

1. Request the search page with the user's query.
2. Read the result-data URL supplied in its `deep_preload_script` element. Keep DuckDuckGo's supplied parameters and verify the destination and original query.
3. Fetch the result data and decode organic titles, destination URLs and snippets from its JSON-containing envelope.
4. If the response contains the supported arithmetic browser check, calculate the answer and follow its continuation once. A real test calculated **10715** and received results afterward.

The calculation recognizes only a small, bounded set of integer operations and known HTML-fragment serialization lengths. AG1 does not execute the downloaded JavaScript. Unknown checks remain errors. Existing DNS/TLS checks, cancellation, request limits and untrusted evidence retention remain in force. This production flow does not require an open browser, screenshots or a vision model.

“Bypass” here means a working route around the HTML/lite failure observed in testing, including automatic handling of the supported browser calculation. It does not remove all DuckDuckGo restrictions or override You.com's quota. An unresolved challenge still stops requests and triggers AG1's existing five-minute per-service cooldown; that duration is local policy, not a verified DDG reset time.

## Effect on search results

**The main improvement is access to real search results.** The change does not alter DuckDuckGo's underlying index or ranking algorithm. AG1 now applies the candidate selection described below to the returned results.

- The model receives actual titles, links and snippets where it previously received blocked or misleadingly empty searches.
- Destination URLs remain available for page reading, citations and the existing website-icon display.
- A known empty-result marker is recognized directly, avoiding unnecessary HTML/lite fallback requests.
- Search snippets remain excerpts, not a verified reading of the entire page. Outdated documentation, duplicate mirrors and weaker sources can still appear.

Two native AG1 runs completed **15 queries without blocking**: 13 returned results and two deliberately nonexistent phrase searches correctly returned empty. Median time was **807 ms**, with no fixed success padding. The final seven-query run used exactly 14 network requests and stayed on the new route throughout. **39 focused automated checks passed.**

These were short native-service tests, not a sustained request-rate measurement or a 35B answer-quality benchmark. The arithmetic branch was verified by a direct live response, a retained real-challenge parser fixture and a native three-request contract; the two native live batches did not encounter it. See [measured summary](../free-search-recovery-20260914/native-summary.json), [final results](../free-search-recovery-20260914/native-final-results.json) and [full methodology](../free-search-recovery-20260914/REPORT.md).

## Installed quality improvements

The user resumed these improvements after the original overview. The [quality checkpoint](../search-quality-20260914/REPORT.md) is installed:

- **Better selection:** inspect up to 24 candidates from the same provider response before returning up to four. Combine relevance, original engine order, project documentation and requested versions; demote known mirrors and unrequested prerelease pages. The final native queries selected the relevant official urllib3, SQLite, Rust and Node.js pages first.
- **Less repeated work:** cache the candidate pool independently of result count and reuse identical concurrent requests. Current/empty searches expire after one minute; ordinary successful results retain the existing 15-minute lifetime. Cache age is visible.
- **Fuller page reading:** DOM-based extraction preserves code, tables, entities and links. Query-focused source excerpts use bounded 12,000 / 7,000 / 3,500-character budgets, keep passages in document order and mark omissions. Original network bytes remain untrusted evidence.
- **Truthful failures:** failed fetches, unsupported binary content and unusable placeholder pages carry explicit error outcomes. A cleanup bug that could turn `copy()` into `()` is corrected.

Twelve native queries completed without blocking across two quality checkpoints; each included one correctly empty control. The final six-query median was **893.5 ms**. **54 focused checks passed**, including offline replay of four real documentation pages. [Measured summary](../search-quality-20260914/summary.json).

The bounded 35B received substantially fuller evidence: the MDN page grew from 972 to 6,395 transcript characters and included both header syntax forms missing from the older extract. This does **not** establish better overall answers: both old and new model answers contained incorrect deadline pseudocode. Both print-mode runs also hit a separate coding-completion gate inappropriate for research. That limitation is retained in the report, not counted as a successful whole-harness run.

## What can improve next

Further useful work includes correcting CLI research completion, testing answer accuracy in a clean matched benchmark, expanding relevance/version/language coverage, and supporting more difficult page types. Publication-date verification, PDFs, JavaScript-only pages and arbitrary DDG challenge formats remain outside this checkpoint. The working route does not guarantee permanent or unlimited access.

## Evidence and source map

- [Working DDG implementation and installation](../free-search-recovery-20260914/REPORT.md), including original bytes, request bodies, tests and failed attempts.
- [You.com evaluation](../free-search-recovery-20260914/YOU-COM.md): 12 calls tested; useful results but loose phrase matching and some outdated top pages. It was not configured as AG1's default.
- [Earlier request-flow/rate-limit investigation](../duckduckgo-request-flow-20260914/REPORT.md), [parser/error repair](../duckduckgo-fix-20260914/REPORT.md), and [original failure diagnosis](../duckduckgo-audit-20260914/REPORT.md). Their blocked outcomes describe earlier checkpoints.
- [Core result and arithmetic decoder](../../../product/crates/ag1-core/src/tools/websearch/duckduckgo_web.rs).
- [Native DDG request flow](../../../product/crates/ag1-platform/src/web_tools/search/ddg_web.rs), [provider selection/cache](../../../product/crates/ag1-platform/src/web_tools/search.rs), and [HTTP transport](../../../product/crates/ag1-platform/src/web_http.rs).
- [Core regression tests](../../../product/crates/ag1-core/tests/duckduckgo_web.rs) and [native contracts/live probes](../../../product/crates/ag1-platform/tests/web_http_contract.rs).

- [Latest quality implementation, installation and bounded model evidence](../search-quality-20260914/REPORT.md).

Existing chat/search follow-up context remains in [task 3](../../../tasks/03-CHAT-CLEANUP.md). Original navigation documents from this documentation edit are preserved in [this archive](navigation-before-20260914.tar.gz).
