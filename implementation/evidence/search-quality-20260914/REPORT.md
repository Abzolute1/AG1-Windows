# Free search quality checkpoint — 14 September 2026

Implemented and installed at the user's request. Production remains DuckDuckGo with no account, key, paid provider or fixed success delay. This improves AG1's retrieval, source selection and page reading; it does not replace DuckDuckGo's index or establish a universal search-engine ranking.

Installed release: `/opt/ag1/releases/7dbee4adb4ba4f1386b96a1a96809573`. Previous release: `/opt/ag1/releases/1505461cd7a74256b2fef7b66dbe13fa`. [Package](package.json), [isolated installation](package-validation.json), [systemwide verification](systemwide-verification.json). All 1,144 payload files, sizes, modes and the installation receipt matched. The final model probe's CLI hash matches the installed CLI.

## Changes and their effect

### Search results

Previously the provider parser and native service trimmed to the requested one to four results before ranking. AG1 can now inspect up to **24 candidates from the same response**, rank/deduplicate them, and only then return up to four. The live DDG responses contained ten usable candidates apiece. No extra search request is needed to use those additional candidates.

Selection combines the provider's original order, topic/title matches, existing source heuristics and known project documentation domains. It demotes known documentation mirrors, unrequested prerelease documentation and explicit version mismatches. An older version remains preferred when the query requests it. An unversioned peer receives preference only when a same-host candidate has at least comparable query coverage. These are transparent heuristics, not proof of correctness or freshness. `scoreReasons`, `candidateCount` and `selectionPolicy` retain the rationale.

Canonical deduplication still ignores fragments and tracking parameters, but now preserves case-sensitive URL paths and query values. Normal transcripts show up to four candidates with longer snippets; tighter context profiles show fewer/shorter results. Selected source URLs continue feeding the existing citation and favicon interface.

### Repeated requests

The cache stores the candidate pool independently of the requested output count. Concurrent calls in one scoped service wait and recheck the cache, so asking for one result and then four reuses the original retrieval. The native contract verifies one network request for two concurrent calls with different counts and shared evidence.

Successful ordinary searches retain the existing 15-minute cache lifetime. Empty results and queries classified as current/recent have a one-minute lifetime. Failed searches are not cached as successes. Provider cooldown still prevents repeated blocked requests. `cacheHit` and `cacheAgeMs` expose reuse. No numeric DDG request quota is asserted and no fixed padding was added.

### Reading source pages

Native `webFetch` now parses HTML with the existing DOM library, selects the primary content region, removes common navigation/hidden/script elements, decodes entities and preserves Markdown structure, code indentation, tables and links. Relative HTTP(S) links resolve against the final page URL; embedded credentials and executable link schemes are omitted. Plain text/Markdown are preserved as text instead of being stripped as HTML.

The model receives a **bounded source excerpt**, in addition to the short assessment. Budgets are 12,000 / 7,000 / 3,500 characters for normal/tight/ultra contexts. Excerpt selection weights query terms by their rarity in the page, considers neighboring passages and restores document order. It recognizes identifiers through Markdown escaping. Complete paragraphs, tables and fenced code are retained when they fit; omission markers and `truncated` expose incomplete reads. Oversized individual blocks can still be clipped. Original network bytes remain available as untrusted evidence.

The old cleanup could turn `copy()` into `()`; it now preserves that API and ordinary prose about copying while handling copy-button labels. Explicit transport failures, unsupported binary content and placeholder/interstitial pages now carry `isError`, so the existing UI can classify them as failures instead of completed source reads.

No downloaded JavaScript is executed. The existing DDG arithmetic handling, public-address checks, DNS pinning, TLS validation, credential boundaries, cancellation and response limits remain in place. No renderer source, model profile, captured launch plan, weights or protected process-host binary changed.

## Native live checks

Two six-query runs completed without blocking. Each returned real results for five queries and correctly returned zero for a nonexistent quoted phrase. Four selected pages were fetched in each run. The second run's median search latency was **893.5 ms**. These are short quality probes, not a throughput/rate-limit measurement.

| Final query topic | Candidates / returned | First result | Search time |
|---|---:|---|---:|
| urllib3 retry/backoff | 10 / 4 | Official urllib3 utility reference | 969 ms |
| SQLite checkpoint / SQLITE_BUSY | 10 / 4 | Official SQLite checkpoint API | 863 ms |
| Rust Iterator / collect | 10 / 4 | Official Iterator trait reference | 1,255 ms |
| Node AbortSignal timeout | 10 / 4 | Official globals reference | 759 ms |
| Oslo airport train tickets | 10 / 4 | Flytoget tickets | 924 ms |
| Nonexistent quoted phrase | 0 / 0 | Correctly empty | 727 ms |

The first run exposed overly aggressive reranking: a SQLite mirror and Node's child-process page outranked the better engine choices. Source preference and provider-order weighting were corrected. The final run put all four relevant official documentation pages first. [Initial results](live-results.json), [final results](live-final-results.json), [raw final responses](live-final-responses.json).

The final passage-weighting adjustment was tested by **offline replay of those same retained bodies**, avoiding another network batch. The replay verifies that urllib3 `allowed_methods`, `backoff_factor` and `DEFAULT_ALLOWED_METHODS`, SQLite `SQLITE_BUSY` and checkpoint mode, Rust `collect`/`FromIterator`, and Node `AbortSignal.timeout` reach the bounded output. [Final extracted passages](page-replay.json), [replay log](page-replay.log).

## Bounded 35B evidence and limitations

The existing 35B model ran sequentially on the old installed CLI and final CLI, with the same research prompt, captured model/runtime settings, isolated state and an 18-generation / 240-second cap per attempt. Each actual inference run made two successful searches and two successful official-page reads. No screens were captured.

| Page supplied to model | Old transcript | Final transcript | Material difference |
|---|---:|---:|---|
| urllib3 Retry | 1,084 characters | 12,198 characters | More parameters, examples and behavior survive extraction |
| MDN Retry-After | 972 characters | 6,395 characters | Both literal syntax forms appear in the final evidence; neither appeared in the old extract |

[Prompt](model-prompt.txt), [evidence comparison](model-evidence-comparison.json), [old answer](model-before-answer.md), [final answer](model-final-answer.md), [old messages](model-before-messages.json), [final messages](model-final-messages.json).

**This demonstrates better evidence delivery, not better overall model correctness.** Both answers gave the correct allowed-method set and header forms. Both produced flawed deadline pseudocode. The final answer also retried an unsafe POST timeout in its pseudocode, contradicting its prose, used an undefined variable and failed to bound request duration. Those are failures, not accepted retry-policy implementations.

Both print-mode runs produced substantive answers but then encountered a separate coding-completion gate requiring file mutations/tests for this read-only research task, and exited 1. They are not successful whole-harness completion results. An intervening JSONL setup attempt never produced an assistant/tool message and was stopped at 240 seconds; it is excluded from quality comparison. No cause is asserted for that stalled setup. [Old run](model-before-run.json), [stalled setup](model-after-run.json), [final run](model-final-run.json). The final inference binary matches the installed package. All owned probe processes were stopped; the separate endurance programme remains paused.

## Verification and retained differences

**54 focused checks passed:** 25 core parsing/search/extraction checks, 23 native HTTP contracts, three native session checks, two native search units and one four-page offline replay. [Core](core-tests-final2.log), [HTTP](native-tests-final.log), [sessions](session-tests-corrected.log), [units](search-units-final.log), [replay](page-replay.log). Clippy completed with surrounding-code warnings and no diagnostics in the new production modules. [Clippy](clippy-final.log).

The untouched frozen core oracle suite was run against both pre-change source in an isolated copy and final source. Before: 26 passed / two existing failures. After: 25 passed / three failures. The additional mismatch is intentional: the old exact compact HTML projection has been replaced by DOM extraction and source excerpts. The frozen fixture and assertions were not rewritten. Dedicated new contracts test the new behavior. [Before](frozen-core-before.log), [after](frozen-core-final.log).

The broader native cancellation check exposed a fixture defect inherited from the earlier switch to POST: it read headers only, then mistook the pending request body for a connection that had not closed on cancellation. The fixture now drains the bounded Content-Length body before checking EOF. The cancellation deadline and closed-socket assertions remain intact. All three native web session checks pass. [Initial failure](session-tests-final.log), [original test source](session-contract-before.rs).

Initial attempts and failures remain retained, including a table test that needed to account for valid Markdown escaping, contracts updated for candidate-cache semantics, and the first package attempt rejected because test-source edits overlapped its input-fingerprint check. The final package was built after source edits settled and passed the installer and payload verification. No failed build was activated.

[Exact pre-change source/navigation archive](source-before.tar.gz), [task patch](changes.patch), [final source hashes](source-after.json), [protected hashes](protected-hashes.json), [measured summary](summary.json). Existing historical evidence remains unchanged.

## Remaining improvements

- Correct research/coding classification in the CLI completion path and establish a clean matched model benchmark before claiming answer-quality gains or superiority over another harness.
- Improve model reasoning/verification for executable policies. More source text alone does not fix unsafe or internally inconsistent pseudocode.
- Expand relevance fixtures to more languages, niche topics, news and explicitly requested versions. The domain map and mirror list are deliberately limited; publication-date verification and strict phrase semantics are not implemented here.
- Improve page-type coverage: oversized single blocks, unusually structured pages, PDFs and JavaScript-only applications remain limitations. HTML cleanup may omit metadata outside the chosen content region.
- DuckDuckGo may change its endpoint, result envelope or challenges. This is a verified working checkpoint, not an unlimited-access guarantee.
