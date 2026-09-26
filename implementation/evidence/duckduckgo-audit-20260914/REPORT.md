# DuckDuckGo integration diagnosis — 14 September 2026

The DuckDuckGo integration was a confirmed search failure point in the bounded 35B comparison. All 16 saved HTTP responses across eight searches (three normal-toolset searches and five restricted-toolset searches) were HTTP 202 human/bot challenge pages. None contained result__a anchors. AG1 recorded every provider attempt as ok with zero results and every search tool result as isError:false. This establishes blocked automated access plus incorrect failure classification, not inferior DuckDuckGo result quality.

## Evidence and method

Read the current native search/HTTP/parser implementation and the original comparison journals. Read original evidence blobs using read-only SQLite connections. Verified both blob SHA-256 and decoded response body SHA-256 for all 16 responses. See [response-audit.json](response-audit.json) for exact original paths, hashes, URLs, redirects and reported outcomes; [sample-response.html](sample-response.html) retains one challenge body. No new network requests, model runs, screen captures, builds, configuration changes or product edits were needed. This establishes the recorded comparison failure, not whether access remains blocked at this moment.

## Current execution path

- Default auto mode selects DuckDuckGo when no Brave credential is configured. Exa, Tavily and SerpAPI names are accepted but their native adapters are explicitly unimplemented and skipped.
- Query planning may append documentation/freshness terms. Results default to three and are capped at four.
- Search sends GET requests with User-Agent Mozilla/5.0 and Accept text/html,application/xhtml+xml through the native DNS-pinned HTTP transport. Each endpoint attempt has an eight-second cap.
- The first endpoint is https://html.duckduckgo.com/html/. The endpoint labelled duckduckgo-lite is actually https://duckduckgo.com/html/. In every recorded second attempt it redirected to the first endpoint; there was no distinct lite backend.
- Transport accepts all successful HTTP statuses, including 202. The search parser looks for result__a anchors and returns an empty vector when none match, without detecting challenge pages. The provider then records status ok.
- When results are empty, the projection may insert static official-documentation suggestions. Otherwise it tells the model No results found. Fallback origin exists in details but is not clearly explained in the normal text. This makes blocked access look like an ordinary unsuccessful search, or a discovered source.
- Empty provider responses can be cached for 60 seconds; the sampled calls were cache misses, so cache reuse was not the cause here.

## Additional code weaknesses, not demonstrated causes of these 16 failures

The regex requires class before href and supports result__a markup only. Equivalent attribute ordering or actual lite result-link markup can be missed. These responses contained no real result anchors, so parser attribute ordering did not cause the observed failure. Repairing that regex alone would not recover results from a human challenge page.

## Implication and repair priorities

Detect and classify blocked/challenge, HTTP, parse-format and genuine empty-result outcomes separately; propagate failure clearly to model and UI; stop repeating equivalent redirected endpoints; keep static suggestions visibly separate from discovered results. Add realistic recorded-response coverage for those cases and robust HTML parsing. Then evaluate whether permitted DuckDuckGo access is sufficiently reliable or an official search API is needed. Improving error handling does not guarantee DuckDuckGo will permit automated searches.

The separate webFetch extraction defect and coding-task failures remain separate issues; this diagnosis does not explain the entire AG1/OpenCode performance gap.
