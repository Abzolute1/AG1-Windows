# Flash-Next configuration summary

> Sanitized technical document. Personal paths, machine identifiers and application-specific records have been removed. Measurements are historical and hardware-dependent; they are not portable performance guarantees. Referenced raw evidence and source bundles have not been sanitized or included. This document alone is not a complete executable reproduction package.

The tested combined candidate uses a patched private-mapping loader and masked-attention implementation, with stock ngram-mod drafting (maximum/minimum/match: 16/2/24) and twelve verification threads. The measured configuration retains 90,000 requested context, Q8 K/V, 42 CPU expert layers and 1024 batch/micro-batch.

These are experimental parameters, not general hardware recommendations. The private-mapping switches are specific to the matching patched build. Preserve ABI compatibility across its executable and libraries.

Performance, memory, startup and recovery tradeoffs belong in the performance report. This sanitized document makes no claim about any particular application installation or deployment.
