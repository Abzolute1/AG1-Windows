# DuckDuckGo request flow and rate-limit investigation — 14 September 2026

Installed request improvements at `/opt/ag1/releases/229d91b0976544c58c9f4661ee9e0542`. All 1,144 payload files passed hash, size and mode checks; package receipt and installed CLI smoke check passed. [Installation verification](systemwide-verification.json). Reopen AG1 to load this version.

**Outcome: real results were recovered, but reliable DDG-only searching remains unresolved.** Neither POST, waiting, an isolated browser nor an accepted verification challenge established a permanent way around blocking. No promise of unlimited or never-blocked searches is supported by these results.

## Delivered behavior

- Submit DDG HTML/lite searches through their normal URL-encoded POST form, including the empty `b` submit field. The earlier GET implementation was challenged while this POST flow subsequently returned real results.
- Preserve exact search terms, including quotes, plus signs and domain restrictions. Retain both the logical query URL and actual initial method/target in network evidence.
- Preserve POST for same-origin 307/308 redirects, use GET after 301/302/303, and reject cross-origin redirects while a search form remains attached. Existing DNS/TLS authorization, cancellation and limits remain enforced.
- Serialize public DDG searches across native services in the same process. **No fixed delay between successful searches.** The experimental eight-second interval was removed after the user asked for evidence of the real quota. Existing bounded cache and per-service five-minute block cooldown remain. That cooldown is AG1's policy, not DDG's documented reset time.

DuckDuckGo remains the only default engine. No new dependency, proxy service, alternate engine or production CAPTCHA solver was added. This builds on the earlier [parser/error repair](../duckduckgo-fix-20260914/REPORT.md).

## Rate limits

No verified HTML/lite requests-per-second or requests-per-minute quota was found. DDG acknowledges rate-limit functions without publishing a numeric quota in the documentation checked. SearXNG's maintainers explicitly describe incomplete knowledge of the blocking mechanism. One advertised one-request/second limit was traced to an MCP wrapper's own constants, not a DDG contract. [Primary sources and findings](RATE-LIMITS.md).

The retained browser challenge response supplied no `Retry-After` or rate-limit quota headers. A blocked first request after more than five minutes does not establish a sustainable request rate. There is no evidence that eight seconds is necessary or sufficient, so it is not shipped.

## Live outcomes

| Probe | Observed outcome |
|---|---|
| Direct form POST | HTTP 200, eight result links, no challenge; [response](form-post.json), [HTML](form-post.html). |
| Native POST, initial sequence | Two successful searches, four parsed results each, then HTTP 202 challenge on the third. Useful top sources included urllib3's utility reference and Rust's ownership chapter. [Results](live-results.json), [raw responses](live-responses.json). |
| Native after more than five minutes, experimental pacing enabled | First request challenged; stopped. Pacing effectiveness could not be measured because there was no second request. [Results](live-results-paced.json). |
| Isolated Chromium HTML form | Challenged. [Original metadata](browser-probe-before-verification.json), [original HTML](browser-search-before-verification.html). |
| Isolated Chromium main JavaScript search | Human challenge, no results. [Response log and visible text](browser-main-probe.json). |
| Challenge submission | Selected the ducks in the isolated browser; DDG returned HTTP 200 and its verification-success page. [Accepted response](first-verification-probe.json), [HTML](first-verification-response.html). |
| Native immediately after accepted verification | Still challenged. [Results](live-results-after-verification.json). |
| Later browser and native follow-up | Browser returned eight links. Native returned four results for the verified urllib3 query, then a challenge for Rust. These probes overlapped, so they are not a controlled throughput experiment. [Browser](browser-probe.json), [native](live-results-verification-settled.json). |

Across the four native live invocations, 3 of 7 actual search attempts returned results. This tiny, adaptive sample is not a reliability estimate. Each native invocation stopped at its first challenge. Successful verification did not establish unrestricted access or a general reset interval. Query scope, timing, request fingerprint and other server-side inputs remain unresolved causes.

Only the isolated browser's challenge panel was captured to inspect its images. The user's desktop and YouTube window were not captured or controlled. No GPU/model run or endurance test was performed.

## Final verification

22 relevant automated checks passed:

- 20 native HTTP contracts, including exact form encoding, redirect methods, cross-origin rejection and existing transport/search boundaries: [log](native-tests-final.log).
- 1 search-serialization unit proving an uncontended call resolves without timer padding, while a queued call respects cancellation and deadline: [log](serialization-test.log).
- 1 native session contract covering search/fetch evidence and current context pressure: [log](session-test-final.log).

The ignored live contract was invoked separately four times and validates result/error classification; a passing test does not mean the search succeeded. The outcome counts above are authoritative for live access.

Final platform Clippy completed with 73 surrounding-code warnings and no diagnostics in the changed request/search files: [log](clippy-final.log). The first pass found one new range-pattern style warning; it was corrected. Frontend/core parser source did not change in this follow-up, so their earlier focused coverage was not repeated. Release compilation, package audits and isolated installation passed: [package](package.json), [validation](package-validation.json).

Final archive: `.local/p14-packages/368c142fdc124df28c38a26080c4faa6/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `9c5631902ed180fa2ccdc48004d741b6442c06510312efa2ce1b969dfe847e16`.

Protected model profiles, runtime capture and original bound process host remain unchanged: [hashes](protected-hashes.json). [Final source hashes](source-after.json), [task-only patch](changes.patch) and [original source archive](source-before.tar.gz) preserve exact custody. Navigation originals are retained in [their archive](navigation-before.tar.gz). Historical comparison expectations were not rewritten; this is not broad release acceptance.

## Preserved attempts

The initial live-evidence test edit omitted a trait import and failed to compile; it was corrected before final checks. The first challenge interaction was intercepted by the page's overlay; its [error](verification-interaction-error.json) is retained. The successful submission used the form's existing label/button DOM activation. A subsequent attempt expected another challenge but found none and timed out before the next step; [record](verification-probe.json). The separate later browser probe captured the actual successful results instead.

Two earlier packages containing the experimental eight-second interval were superseded without systemwide installation: [first package](superseded-package/package.json), [second package](superseded-paced-package/package.json). The final installed package has no fixed success padding. The previous installed release remains `/opt/ag1/releases/7bb6bc55e5cc4d06a5c3e554e19f1b54`.

The separate webFetch extraction/error-presentation findings, 35B/OpenCode comparison and broader chat/artifact closeout remain outside this follow-up. No reliable automatic solution to DDG's recurring challenges has been established.
