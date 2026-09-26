Now I have all three sources. Here is the answer:

---

## Production Python HTTP Client Retry Policy

### When Automatic Retries Are Unsafe

Retries are unsafe whenever the request body may have reached the server. urllib3 documents that `read` errors occur "after the request was sent to the server, so the request may have side-effects" ([urllib3.util.Retry](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry), `read` parameter). Similarly, `other` errors "might be raised after the request was sent to the server, so the request might have side-effects" ([urllib3.util.Retry](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry), `other` parameter). In the order-creation scenario, a timeout after the body was transmitted means the order may already exist; retrying without an idempotency key risks a duplicate order. By contrast, `connect` errors are "raised before the request is sent" and are safe to retry ([urllib3.util.Retry](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry), `connect` parameter).

### What an Idempotency Key Changes

MDN documents the `Idempotency-Key` request header, which lets the server deduplicate retries and return the original result without creating a second resource ([MDN Idempotency-Key](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Idempotency-Key)). With a key, a retried POST becomes semantically idempotent: the server either creates the order once or returns the existing one. Without it, a retried POST is a second order.

### Both Forms of Retry-After and Past Dates

MDN specifies two forms ([MDN Retry-After](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Retry-After)):

1. **`Retry-After: <delay-seconds>`** — a non-negative integer of seconds to wait after receiving the response.
2. **`Retry-After: <http-date>`** — an absolute HTTP-date after which to retry (e.g., `Wed, 21 Oct 2015 07:28:00 GMT`).

If the http-date is in the past, the client should retry immediately (zero wait). If the delay-seconds value is zero, also retry immediately.

### Why an Elapsed-Time Deadline Needs a Monotonic Clock

A 12-second elapsed-time budget must use `time.monotonic()` (Python stdlib) because wall-clock time (`time.time()`) can jump backward due to NTP adjustments, daylight saving, or manual changes, causing a retry to appear within budget when it has actually exceeded the real elapsed time. `time.monotonic()` is guaranteed non-decreasing.

### urllib3 Retry Default `allowed_methods` and POST

urllib3's default is `DEFAULT_ALLOWED_METHODS = frozenset({'DELETE', 'GET', 'HEAD', 'OPTIONS', 'PUT', 'TRACE'})` ([urllib3.util.Retry](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry.DEFAULT_ALLOWED_METHODS)). **`POST` is not in this set.** The docs state: "By default, we only retry on methods which are considered to be idempotent" ([urllib3.util.Retry](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry), `allowed_methods` parameter).

**Claim "urllib3 retries POST by default" is false.** POST is explicitly excluded from the default allowed set. To retry POST, you must pass `allowed_methods=['POST', ...]` or `allowed_methods=None` to retry on any verb.

### Bounded Retry Decision (Pseudocode)

```python
import time, http.client
from urllib3.util import Retry

def create_order(order_data, idempotency_key):
    deadline = time.monotonic() + 12.0          # 12s budget, monotonic
    retry_cfg = Retry(
        total=3,
        allowed_methods=["POST"],                # POST is NOT default
        status_forcelist=[429, 503],
        backoff_factor=0.5,
        backoff_jitter=0.1,
        respect_retry_after_header=True,
    )
    attempts = 0
    while True:
        conn = http.client.HTTPSConnection(host, timeout=4)
        conn.request("POST", path, body=order_data,
                     headers={"Idempotency-Key": idempotency_key})
        try:
            resp = conn.getresponse()
        except http.client.HTTPException:
            if time.monotonic() >= deadline:
                raise
            attempts += 1
            if attempts >= retry_cfg.total:
                raise
            retry_cfg.sleep()
            continue

        if resp.status == 429:
            retry_after = resp.getheader("Retry-After")
            if retry_after:
                try:
                    wait = float(retry_after)          # delay-seconds
                except ValueError:
                    # http-date form: parse, clamp past dates to 0
                    wait = max(0, parse_http_date(retry_after) - time.time())
                wait = min(wait, retry_cfg.retry_after_max)
                remaining = deadline - time.monotonic()
                sleep_time = min(wait, remaining)
                if sleep_time <= 0:
                    break                             # budget exhausted
                time.sleep(sleep_time)
            else:
                retry_cfg.sleep()                     # exponential backoff
        elif resp.status >= 500:
            if time.monotonic() >= deadline:
                break
            attempts += 1
            if attempts >= retry_cfg.total:
                break
            retry_cfg.sleep()
        else:
            return resp                               # success / client error
```

**Key distinctions from urllib3 defaults:** The policy explicitly adds `POST` to `allowed_methods` (not in the default), sets `status_forcelist=[429, 503]` (default is `None`, no retries on status codes), adds `backoff_factor=0.5` with jitter (default is 0, no backoff), and enforces an absolute monotonic deadline (urllib3 has no built-in elapsed-time cap).



Research complete. Sources consulted:

| Source | URL |
|--------|-----|
| urllib3.util.Retry | https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html |
| MDN Retry-After | https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Retry-After |
| Python urllib.request | https://docs.python.org/3/library/urllib.request.html |
