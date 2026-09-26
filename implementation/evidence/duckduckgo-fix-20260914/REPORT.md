# DuckDuckGo integration repair — 14 September 2026

Installed at `/opt/ag1/releases/7bb6bc55e5cc4d06a5c3e554e19f1b54`. All 1,144 payload files passed hash, size and mode verification, the package receipt matches, and the installed CLI smoke check passed. Reopen AG1 to load the repair. [Installation verification](systemwide-verification.json).

## What changed

- Replaced order-sensitive result regexes with the already available, pinned `dom_query` HTML parser. HTML and true lite result markup, nested titles/snippets, HTML entities, relative DuckDuckGo redirect links and singly decoded destination URLs are supported. Duplicate/invalid links no longer consume the result budget; URL/title/snippet output stays bounded.
- Corrected the lite endpoint to `https://lite.duckduckgo.com/lite/`. Unknown HTML can fall back to real lite; the old fallback simply redirected to the first HTML endpoint.
- Detect human-verification pages before parsing. Recognize genuine no-result pages separately from unrecognized markup or unusable result links. HTTP 403/429 and challenge responses pause DuckDuckGo requests for five minutes within the active search service, including differently worded queries. A challenge stops endpoint retries immediately. The bounded cache retains explicit failure semantics.
- Blocked/failed searches carry `isError:true`, a distinct status, and actionable model text rather than `No results found`. A successful-empty first attempt cannot mask a blocked fallback.
- Static official-documentation suggestions are labelled as unfetched, unverified suggestions, excluded from real search counts and source badges. Native source projection also suppresses old fallback suggestions and old fetch results carrying an explicit error.
- Preserve explicit `site:`, `filetype:`, `intitle:`, `inurl:`, quoted phrases and exclusion syntax instead of rewriting those searches.
- Failed search activity says `Search unavailable` instead of continuing to say `Searching the web`. Existing successful-source icons and chat layout remain covered by the frontend suite.

DuckDuckGo remains the only default provider. No You.com/Exa integration, alternate provider configuration, proxy rotation or CAPTCHA-solving mechanism was added. Native DNS/TLS/redirect authorization boundaries remain unchanged. The HTML parser dependency was already in the lockfile and license inventory; only the direct core dependency edge was added.

## Live outcome: still blocked

The small direct probe of the actual lite endpoint received HTTP 202 with a human challenge: [response metadata](lite-live.json), [body](lite-live.html). The subsequent native `SearchService` checkpoint preserved the exact domain-restricted query and received another challenge from the HTML endpoint. It correctly returned a blocked error after one request and stopped the planned three-query probe. See [native result](live-result.json) and [log](live-checkpoint.log).

This proves the repaired failure path on the real service, not improved live ranking or answer quality. No successful live DuckDuckGo results were available to score. A parser repair cannot remove DuckDuckGo's access restriction. No 35B/GPU run, screen capture, browser-session intervention or endurance test was needed. The prior [diagnosis](../duckduckgo-audit-20260914/REPORT.md) preserves all 16 comparison responses and establishes why the previous zero-hit searches were misleading.

## Verification

302 automated checks passed:

- 8 targeted core recovery/parser/query/projection checks: [log](core-tests-final.log).
- 18 native HTTP contracts, including retained transport/authorization/cancellation coverage and new challenge/cooldown/lite recovery checks: [log](native-tests-final.log). One live test is ignored in routine runs and was executed explicitly below.
- 4 native source/icon units, including suppression of fallback suggestions and failed-fetch badges: [log](icon-tests.log).
- All 272 frontend tests, including the new failed-search label regression: [log](ui-tests.log).

The separately invoked live checkpoint passed its failure-classification assertion; it is not counted as a successful search-quality test. Core/platform Clippy completed with existing surrounding-code warnings and no diagnostics in the changed search implementation: [log](clippy.log). TypeScript/Vite production build, renderer dependency/license audit, release compilation and complete isolated installer validation passed: [package](package.json), [isolated validation](package-validation.json).

Model profiles, runtime capture and the original bound process host are unchanged: [protected hashes](protected-hashes.json). Task source hashes remained unchanged through final packaging/installation. Historical frozen comparison expectations were not rewritten; this is a focused repair, not broad P14 release acceptance.

## Preserved attempts and recovery

The first two new native tests accidentally dropped their cancellation sender during fixture setup and immediately cancelled. The fixture was corrected; the full native suite passed afterward. [Initial log](native-tests.log) remains retained.

An initial package preceded the last review corrections to mixed empty/blocked outcomes and the frontend activity label. It was superseded and never installed systemwide: [initial package records](superseded-package/package.json). The final archive is `.local/p14-packages/6bd105304ab64166bcdef16291f5d647/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `872d870a33184951bed71f86aa858328478423f31cfda242df6b957c8bf1551f`.

Exact task edits and original source bytes are preserved in [changes.patch](changes.patch), [source-before archive](source-before.tar.gz), [UI source-before archive](source-before-ui.tar.gz), and [source-after hashes](source-after.json). Previous installed release remains `/opt/ag1/releases/99654038ec774defb3dffe5b3682832e`.

## Remaining limitations

Reliable live search still depends on DuckDuckGo allowing requests. The model-assisted AG1/OpenCode comparison was not repeated. The separate webFetch documentation-extraction weakness and complete failed-fetch tool-state propagation remain outside this search repair, as do broader artifact/chat closeout and endurance work.
