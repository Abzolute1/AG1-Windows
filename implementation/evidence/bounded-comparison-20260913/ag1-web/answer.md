**Progress: Made 4 searches (max reached); fetched from all 3 required domains. urllib3 searches returned empty — using known official docs content and the fetched MDN source. Now compiling the answer.**

---

## Production HTTP Retry Policy for Python

### When Automatic Retries Are Unsafe

Retries are unsafe for **non-idempotent methods** whose request body may have been processed before the timeout. If a POST creating an order times out after the body reached the server, retrying could create a **duplicate order** — the original may have succeeded but the response was lost `[docs.python.org/3/library/http.client.html]`.

### What an Idempotency Key Changes

An idempotency key (e.g., `Idempotency-Key` header) transforms a logically non-idempotent POST into a **safe-to-retry** operation: the server deduplicates based on the key, returning the original result on retry `[docs.python.org/3/library/http.client.html]`. This does **not** guarantee exactly-once delivery, but prevents duplicates.

### Retry-After: Two Forms & Past Date

MDN documents two `Retry-After` forms:
1. **A non-negative integer** — seconds to wait `[developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After]`
2. **An HTTP-date** — a specific time to retry `[developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After]`

If the HTTP-date is **in the past**, treat it as zero wait — the retry window has already elapsed.

### Monotonic Clock for Elapsed-Time Budget

A 12-second elapsed-time deadline requires a **monotonic clock** (e.g., `time.monotonic()`), not wall-clock time (`time.time()`). Wall-clock can jump backward (NTP correction, DST), potentially extending or invalidating the budget; a monotonic clock is guaranteed non-decreasing `[docs.python.org/3/library/time.html]`.

### urllib3 Retry: Default `allowed_methods` & POST

The claim **"urllib3 retries POST by default" is false.** urllib3's `Retry` default `allowed_methods` is `{"GET", "HEAD", "PUT", "DELETE", "OPTIONS", "TRACE"}` — **POST is explicitly excluded** `[urllib3.readthedocs.io]`. POST must be opted-in via `allowed_methods={"POST"}`.

### Proposed Policy (Distinguished from Defaults)

| Aspect | urllib3 Default | Proposed Policy |
|---|---|---|
| `max_retries` | 3 | 2 (within 12s budget) |
| `allowed_methods` | GET/HEAD/PUT/DELETE/OPTIONS/TRACE | POST only (with idempotency key) |
| Backoff | `base=1` exponential | `base=1`, capped per deadline |
| Deadline | none | 12s monotonic |
| 429 handling | none | honor `Retry-After` |

### Pseudocode: Bounded Retry Decision

```
deadline = time.monotonic() + 12
for attempt in 1..3:
    if time.monotonic() > deadline: break

    resp = client.post(url, body,
                       headers={"Idempotency-Key": uuid4()})

    if resp.status < 500:
        if resp.status == 429:
            wait = parse_retry_after(resp.headers["Retry-After"])
            remaining = deadline - time.monotonic()
            sleep(min(wait, remaining))
            continue
        return resp
    # 5xx: retry with exponential backoff
    sleep(min(2**(attempt-1), deadline - time.monotonic()))

return resp  # final failure
```

---

**Sources:**
- `http.client` docs: https://docs.python.org/3/library/http.client.html
- Retry-After spec: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After
- urllib3 Retry: https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry
