I searched for and read the official Python docs page at **https://docs.python.org/3/library/time.html** (fetched via `webFetch`).

The page states that `time.monotonic()` returns *"a clock that cannot go backwards"* and is *"not affected by system clock updates."*

**Why monotonic time is useful for measuring elapsed time:**

`time.monotonic()` is designed specifically for measuring durations because it is immune to system clock adjustments. Unlike `time.time()` (wall-clock time), which can jump forward or backward when NTP syncs, daylight-saving changes, or manual edits occur, monotonic time only ever moves forward. This guarantees that subtracting two readings always yields a correct, non-negative elapsed duration — no matter what happens to the system clock between calls.
