I've completed the search and read the page. Here's the explanation:

---

**Why monotonic time is useful for measuring elapsed time**

`time.monotonic()` returns a clock that **never goes backwards** and is **unaffected by system clock updates** (e.g., NTP adjustments, manual changes, or leap seconds). This means the difference between two `monotonic()` calls always yields a consistent, reliable elapsed duration, unlike `time.time()` which can jump due to wall-clock corrections. It is also suspend-aware, continuing to advance correctly after sleep or hibernation. The reference point is arbitrary, so only differences between two calls are meaningful.

**Cited page:** [time — Time access and conversions](https://docs.python.org/3/library/time.html) (docs.python.org/3/library/time.html)
