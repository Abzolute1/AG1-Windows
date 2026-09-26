# DuckDuckGo rate-limit investigation — 14 September 2026

No verified requests-per-second or requests-per-minute quota was found for the HTML/lite web-search endpoints used by AG1. The eight-second interval initially introduced was a precaution, not a measured or published provider limit, and has been removed from the final source. Public DDG calls remain serialized across search services in the process, without a fixed delay after success. Existing bounded caching and five-minute failure cooldown remain; that cooldown is AG1 policy, not a claim about DDG's reset time.

## Primary sources checked

- [DuckDuckGo's performance documentation](https://duckduckgo.com/duckduckgo-help-pages/privacy/atb) acknowledges fraud detection and rate-limit functions but gives no numeric search quota.
- [DuckDuckGo's HTML/lite documentation](https://duckduckgo.com/duckduckgo-help-pages/features/non-javascript) documents those interfaces without a numeric automation quota.
- [SearXNG's DDG engine documentation](https://docs.searxng.org/dev/engines/online/duckduckgo.html) says the blocking mechanism is not fully known. Its description of an approximately one-hour sliding window is historical developer observation, not a current contractual reset interval. It also documents request-form/header effects on detection.
- A search result advertising one request/second and 15,000/month was traced to [this MCP wrapper's source](https://github.com/zhsama/duckduckgo-mcp-server/blob/main/src/index.ts). Those are its own CONFIG.rateLimit constants and local checkRateLimit implementation, not evidence of DDG's actual limits.

## Local evidence and its limits

Two native POST searches succeeded about two seconds apart; the third received an HTTP 202 human challenge. A later first request, more than five minutes after that run, still received a challenge. This is insufficient to estimate a sustainable throughput or infer that the third request crossed a fixed quota. Both tests reused the connection and queries over time rather than controlling every detection input.

The isolated Chromium HTML-form probe also received a challenge. Its retained response headers contain no Retry-After or rate-limit quota headers: [metadata](browser-probe-before-verification.json). The main JavaScript page likewise displayed a human challenge: [metadata and visible text](browser-main-probe.json). These observations do not establish an exact detection cause or reset period.

Removing artificial padding reduces local latency; it does not demonstrate that DDG will accept more traffic. A defensible throughput measurement requires a working baseline first. Repeatedly varying delays while every first request is challenged cannot identify a requests/minute threshold.
