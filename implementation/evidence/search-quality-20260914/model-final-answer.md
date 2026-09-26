## urllib3 Retry Default `allowed_methods` and POST Safety

**Default set:** `frozenset({'DELETE', 'GET', 'HEAD', 'OPTIONS', 'PUT', 'TRACE'})` — POST is **not included** ([urllib3 docs](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html)).

**Why retrying a timed-out order-creation POST is unsafe:** POST is *not* idempotent. If the server actually processed the order but the response was lost (timeout), a retry creates a **duplicate order**. The idempotent methods (GET, PUT, DELETE, etc.) are safe to retry because repeating them produces the same final state.

---

## Retry-After Forms (MDN)

Two syntactic forms:

1. **`<delay-seconds>`** — a non-negative integer of seconds to wait after receiving the response.
2. **`<http-date>`** — an absolute HTTP-date timestamp after which to retry.

**Handling a past date:** If the `<http-date>` value is earlier than the current time, the retry window has already expired — the client should retry **immediately** rather than waiting.

---

## Pseudocode: 12-Second Overall Retry Deadline (Monotonic Clock)

```python
import time

deadline = time.monotonic() + 12  # absolute deadline, not a counter

while True:
    try:
        response = session.post(url, data=payload)
        break  # success
    except TimeoutError:
        elapsed = time.monotonic() - start
        if time.monotonic() >= deadline:
            raise  # 12 s exceeded; do NOT retry
        time.sleep(min(backoff, deadline - time.monotonic()))

    except ResponseWithRetryAfter as exc:
        retry_after = exc.parse_retry_after()  # seconds or http-date
        if isinstance(retry_after, datetime):      # http-date form
            wait = max(0, (retry_after - utcnow()).total_seconds())
        else:                                        # delay-seconds form
            wait = retry_after
        wait = min(wait, deadline - time.monotonic())
        if wait <= 0:
            continue  # past-date → retry immediately
        if time.monotonic() + wait >= deadline:
            raise
        time.sleep(wait)
```

**How this differs from library defaults:**
- urllib3's built-in retry tracks *attempts* (via `total`), not a wall-clock deadline.
- urllib3 caps `Retry-After` values at `retry_after_max` (default 21 600 s) but does not enforce an overall budget ([urllib3 docs](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html)).
- This policy uses `time.monotonic()` (guaranteed non-decreasing, immune to wall-clock adjustments) to enforce a hard 12-second ceiling, regardless of backoff or `Retry-After` values.

**Sources:**
- urllib3 Retry defaults: [urllib3.readthedocs.io](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html)
- MDN Retry-After: [developer.mozilla.org](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Retry-After)
