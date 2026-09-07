# Native research transport (P09 in progress)

`web_http` owns HTTP, TLS, DNS and credentials. `web_tools::WebService` executes
approved fetches and `web_tools::SearchService` executes approved searches. Both
use core research projections and the P05 artifact port. P10 will supply their
session lifetime, current policy/epoch, UTC year and cancellation source. The
existing TS desktop still owns live sessions.

Every HTTP hop is checked against the destination policy and connected using only
that hop's resolved addresses. A fresh client prevents connection-pool reuse from
bypassing that decision. Proxies, ambient credentials and cookie persistence are
absent. HTTP(S) URL parsing rejects userinfo. Exact host-owned origin exceptions
allow configured private services; tool arguments cannot grant them. Host-added
CA certificates retain chain and hostname verification. Credentials use explicit
HTTPS hostname allowlists, including subdomains, and are removed on redirects
outside their allowlist. Named credentials are host values; resource-file import
and trust decisions are not implemented by this transport.

Requests have a total deadline (fetch at most 30 seconds; each search attempt at
most 8 seconds), five redirects and at most 32 DNS addresses. Eight requests can
use the transport concurrently. Each decoded entity is limited to 1,000,000 bytes,
including gzip, deflate and Brotli. Cancellation/deadline checks cover DNS, headers
and each body chunk. Dropping an asynchronous OS DNS lookup cannot cancel a lookup
already running in the resolver's blocking pool, but it cannot start a later HTTP
request or publish a tool result.

Fetch caches are scoped to a host service's workspace, session, epoch and immutable
credential/configuration incarnation: 32 entries / 8 MiB, 15-minute TTL. Search
caches use 32 entries / 2 MiB, 15-minute positive and 60-second empty-result TTLs.
Search keys include the original query, requested result count and UTC year. Hint
and prior-fetch-outcome maps each hold at most 256 entries. A host must replace the
service and approval target when configuration/credentials change. Cache response
bodies and hints are ephemeral; acknowledged evidence artifacts and tool outcomes
are durable. P10 must retire an old epoch before admitting another owner.

Search supports DuckDuckGo HTML/lite and Brave. SerpAPI, Tavily and Exa retain the
reference's explicit unsupported-provider attempt followed by DuckDuckGo. An
unavailable provider is not claimed operational. Official documentation fallbacks
are the pinned reference's static URLs and descriptions, not freshly fetched
sources. Search source rankings and `authoritative` labels are reference heuristics,
not proof of factual correctness.

Full decoded response bytes, URL chain, HTTP status and body digest are captured as
untrusted `web-response` artifacts before projection. Search results retain their
provider-response artifact IDs. Cache hits preserve provenance. Neither adapter
creates source-inspection evidence, a verified task claim, or a parent journal
record. The durable executor owns tool intent/result and stable lifecycle events;
its synchronous adapter can invoke the asynchronous web service from a host tool
worker. P10 must not block an asynchronous runtime thread with `block_on`.

## Deliberate reference corrections

- Full IPv6 link-local /10, Teredo/6to4 tunneling, trailing-dot localhost, URL
  userinfo and empty DNS answers fail closed. DNS validation is bound to the
  actual connection, rather than resolving a second time during fetch.
- Credentials require HTTPS; arbitrary workspace environment variables do not
  enable private-address access or discover a secret.
- Cached redirects retain the final URL and actual status. The reference cache
  returns the original URL and status 200 for the second redirected fetch.
- Search cache keys include count/year and preserve query case. A prior short
  search cannot silently restrict a later request for more results.
- Nonpositive/fractional `maxResults` returns an explicit invalid-input error;
  the reference's unconstrained numeric schema can trigger negative/fractional
  array-slice behavior. The pinned registration schema remains recorded.
- Search HTTP errors retain their numeric status and use its canonical reason;
  arbitrary HTTP/1 custom reason phrases are not reproduced.
- Reference UTF-16 slices that split a surrogate are explicitly transported as
  U+FFFD, because native strings and artifact bytes use valid UTF-8.

Captured reference scripts cover 307 research helpers/transcripts, 12 fetch cases,
36 URL policies, seven real compressed responses, 42 full searches and 18 HTML
parser cases. Native tests additionally exercise real TLS, credential stripping,
private redirects, oversized/partial entities, deadlines/cancellation, P05 replay,
cache expiry and provider fallback. The final P09 gate and broader extension/resource
work remain pending; this document is not a phase completion record.
