# Original repository comparison — three models, twelve pairs

Both versions produced correct source in all twelve cases. Rust met the original
evidence requirements in one more case, while using more requests and tool calls.
The results do not establish a consistent improvement in explanation quality or
efficiency.

| Observed outcome | TypeScript | Rust |
| --- | ---: | ---: |
| Correct source contracts | 12/12 | 12/12 |
| Original evidence criteria, after verified measurement corrections | 11/12 | 12/12 |
| Fully supported final explanations | 10/12 | 9/12 |
| Cases with contradicted final claims | 2 | 2 |
| Additional case with unsupported historical assumption | 0 | 1 |
| Model requests | 95 | 119 |
| Generated task tool calls, excluding seeded historical calls | 120 | 143 |

Rust used 19.2% more task tools and 25.3% more requests. These are functional
workload counts, not measured latency or throughput improvements. Some calls add
useful boundary checks; others repeat known information or repair mistakes in the
model's own supplemental checks.

The contradicted TypeScript claims concern string zero being described as falsy
and incorrect examples of the original clamp behavior. The contradicted Rust
claims concern supposedly unchanged null/undefined coercion in the export case
and an inverted-bound clamp result mislabeled as the minimum. A separate Rust
claim about the prior string-input assumption is unsupported; its new throw
description is correct, and undocumented prior caller expectations are unknown.
These explanation findings do not turn correct required-domain source into a
source failure.

The TypeScript evidence miss is the original 35B export case's skipped required
historical retrieval. Raw native residency and cursor-attribution failures remain
retained alongside separately validated corrections based on the actual outgoing
tool messages and authenticated artifact reads. No original threshold was weakened.

This is one fixed-order execution per model, case and adapter. The assembled
prompts differ, including the initial workspace snapshot and native verification
guidance, and adapter names are visible in paths. The interrupted 27B retry pair
also spans execution windows. These conditions prevent a causal claim about the
implementation language. The high-thinking repository results remain separate
from the low-thinking six-task long-horizon comparisons.

The frozen product and package were unchanged throughout the matrix. All 1,780
files from the stopped original matrix remain unchanged. The separately started
feedback workload will conclude with a fresh production preservation audit.
No new eight-hour or 24-hour workload has been started during this continuation;
the earlier failed endurance attempts remain retained.

The complete counts, per-model reviews and artifact bindings are in
[repository-three-model-matrix-review.json](repository-three-model-matrix-review.json).
