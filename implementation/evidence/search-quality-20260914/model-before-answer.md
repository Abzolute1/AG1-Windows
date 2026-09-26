## urllib3 Retry Default `allowed_methods`

The default is `frozenset({'DELETE', 'GET', 'HEAD', 'OPTIONS', 'PUT', 'TRACE'})` ([urllib3 docs](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html)). **POST is NOT included.** urllib3 only retries methods it considers safe/idempotent by default.

## Why Retrying a Timed-Out Order-Creation POST Is Unsafe

A POST that creates an order is **non-idempotent**: the server may have accepted and processed the request even though the client timed out waiting for a response. Retrying would create a **duplicate order**, causing financial harm (double charge) and data corruption. This is why `POST` is excluded from urllib3's default retry set.

## Retry-After Header: Two Forms

MDN documents two forms of the `Retry-After` header:

1. **Delta-seconds** — an integer number of seconds to wait (e.g., `Retry-After: 30`).
2. **HTTP-date** — a specific date/time in RFC 7231 format (e.g., `Retry-After: Fri, 31 Dec 1999 23:59:59 GMT`).

**Handling a past date:** If the HTTP-date value is in the past, treat the delay as zero (retry immediately) or apply a minimal backoff. The server is signaling "now is fine" or the clock has drifted.

## Pseudocode: 12-Second Overall Retry Deadline

```python
import time

MAX_DEADLINE = 12  # seconds (custom policy)
max_retries = 3

deadline = time.monotonic() + MAX_DEADLINE

for attempt in range(max_retries + 1):
    try:
        resp = session.post(url, json=payload, timeout=5)
        if resp.status_code in (429, 503):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break  # exceeded 12s budget
            delay = min(parse_retry_after(resp), remaining)
            time.sleep(max(0, delay))
            continue
        return resp
    except Timeout:
        break  # unsafe to retry non-idempotent POST

return resp  # last response or last exception
```

**Distinction from library defaults:** urllib3's `Retry` class has its own `total` counter and per-attempt backoff, but **no wall-clock deadline** and **does not retry POST by default** (`allowed_methods` excludes `POST`). This policy adds an absolute 12-second monotonic clock cap (`time.monotonic()`) that overrides individual retry attempts, ensuring the overall operation does not exceed the deadline regardless of how many retries the library would permit.

---

**Sources:**
- urllib3 Retry: [urllib3.readthedocs.io](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html)
- Retry-After: [MDN Web Docs](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Retry-After)
