# You.com free search evaluation — 14 September 2026

Twelve sequential `you-search` calls completed without a search error or block. Median measured time was 0.836 seconds (range 0.746–1.255 seconds). This was a direct protocol evaluation using public test queries, not an AG1/35B benchmark or long-term reliability test. [Summary](you-summary.json), [first eight queries](you-quality.json), [four follow-ups](you-quality-followup.json).

The initial Python default User-Agent received HTTP 403 during initialization. Repeating with `Mozilla/5.0` succeeded. The request scripts and both attempt logs are retained. No credentials, account, subscription or payment method were used.

## Useful behavior

- The urllib3 query returned the current utility reference first, with snippets explaining allowed retry methods and status lists. Site-restricted Python and PostgreSQL queries also returned their official domains.
- Broad Rust and Oslo queries included primary sources at rank three, although blogs/tourism content ranked above them.
- Site-restricted follow-ups stayed within the requested domain. Highlight extraction worked on all 31 web results returned by the four follow-up calls.
- The freshness query returned five web and five news results. Publication/update metadata is provided, but this is not proof that every result describes a new event or is accurate.
- All 71 returned web-result rows included a `favicon_url`; no AG1 UI rendering of these You.com results was performed.

## Quality limits

Both fabricated quoted-phrase controls returned five unrelated results instead of an empty answer. Exact quotations therefore cannot be treated as a guaranteed hard filter on this path.

Domain restriction improved source identity but did not guarantee the best page. A Rust follow-up ranked an old Rust-by-Example print page first; an Oslo ticket-price query ranked a 2021 price-change page first. A 20-result broad Rust request returned 16 results and changed the ranking. Applications still need source selection, date checks and page reading before answering.

## Free allowance

[You.com's MCP documentation](https://you.com/docs/build-with-agents/mcp-server) advertises `https://api.you.com/mcp?profile=free` with 100 search queries per day and no credentials. That is a real cap, not unlimited access. No supported exemption or reset/bypass mechanism was found or implemented. No account/IP rotation or paid escalation was attempted.

The live tool schema allows up to 100 results per section per search. Requesting more results can reduce follow-up calls but does not establish better ranking or remove the daily limit. Local caching can avoid spending another request on the same search. [Search-operator documentation](https://you.com/docs/guides/search-operators) describes domain/file-type and inclusion/exclusion filters; the live negative controls above show why behavior must still be tested.

The live free server advertised an additional discovery tool despite the documentation's search-only description. This evaluation called only `you-search`. It did not invoke paid extraction modes or other tools.

You.com was evaluated but not configured as AG1's default: the concurrently discovered DuckDuckGo web flow now supplies working searches without depending on You.com's 100/day allowance. See the [implementation report](REPORT.md).
