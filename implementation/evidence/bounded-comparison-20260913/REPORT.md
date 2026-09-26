# Bounded 35B comparison: AG1 and OpenCode

Completed 13 September 2026. This was a short, sequential diagnostic comparison, not the eight-hour programme or a release-acceptance benchmark.

## Result

**AG1 did not outperform OpenCode in this sample. OpenCode completed more of the requested work, but neither harness passed every requirement.**

| Test | AG1 | OpenCode 1.18.18 |
|---|---|---|
| Stock-ledger repair | **16/18 held-out tests passed**; did not finish by the 360-second cap | **17/18 passed**; reported completion in 235.7 seconds |
| Remaining code defect | Double-counted within-batch stock changes, allowing a negative balance through the API and CLI | Accepted unknown event fields despite explicit strict-schema requirements |
| Protected files | All four preserved | All four preserved |
| Research with normal toolset | No finished answer by the 240-second cap; repeated fetch/extraction attempts | Answer completed in 196.7 seconds, but fetched only two required domains and had deadline bugs |
| Search service | DuckDuckGo returned zero results; a built-in official-source fallback supplied the Python documentation URL | Native Exa searches returned substantive results |
| AG1 visual behavior | Website favicons loaded, active thinking followed work, no observed horizontal overflow | OpenCode was evaluated through its CLI; its UI was not visually compared |

The evidence supports an edge for OpenCode on these tasks, not a general claim that one harness is universally better. Single attempts, different native toolsets/system prompts and search services, and startup differences limit causal conclusions.

## Icons and thinking placement

**The icons really appeared in the requested compact style.** The screenshots show circular website favicons beside muted activity labels, with the current thinking indicator beneath completed work. AG1 retains its own work header and disclosure controls.

- [Actual search row with Python favicon and thinking below it](ag1-web/screenshots/web-thinking-with-icons.png).
- [Normal-toolset run](ag1-default-web/screenshots/web-thinking-with-icons.png).
- [Expanded sources at 1440×900](ag1-web/screenshots/web-sources-1440.png) and [900×600](ag1-web/screenshots/web-sources-900.png).

Images loaded successfully for Python, MDN, urllib3 and Requests. Across **920 recorded native frames** in the retained research/coding runs, the probes found no misplaced active thinking/status, capped inner scrolling during work, or horizontal page overflow. These are observed samples rather than a guarantee about every possible interaction.

The Python icon was on a `webSearch` result row, but the URL came from AG1's **official-source fallback**, not a successful DuckDuckGo result. The native record explicitly reports `searchProvider: official-fallback` and `fallbackUrlsFetched: false`. Loading its favicon does not prove the search service found relevant content.

All screenshots came from the real Linux Tauri/WebKit AG1 application on an authenticated private Xvfb display using the current renderer. No desktop, YouTube, Firefox or ChatGPT screen was captured.

## Research findings

The task required two to four search queries, actual fetching of three official domains, evidence-based explanations of POST/idempotency/Retry-After/monotonic deadlines/urllib3 defaults, citations and bounded pseudocode under 700 words. [Exact prompts and initial scoring plan](plan.json).

### AG1 restricted web/icon probe

The initial search/fetch-only run completed in 100.7 seconds and produced a 408-word answer. It fetched the three requested domains, but made **five searches while claiming four**. It also:

- Generated a fresh idempotency key inside each retry iteration, defeating retry deduplication.
- Omitted a request timeout tied to the remaining deadline, so its pseudocode did not enforce the promised 12-second bound.
- Gave wrong backoff and Retry-After defaults in its comparison table.
- Used inline code-style citation labels that often did not directly substantiate the nearby claim.

[Answer](ag1-web/answer.md), [raw native messages](ag1-web/raw-messages.json), [visual/result record](ag1-web/result.json).

### AG1 normal-toolset diagnostic

Because the icon probe exposed only webSearch and webFetch, an additional run used AG1's normal toolset after OpenCode had stopped. The identical prompt still did not produce a finished answer within four minutes. It made **3 searches, 15 fetches and 8 shell calls**, repeatedly trying to recover missing documentation. This extra attempt is retained and disclosed; it is not a hidden reroll or a statistically paired replicate.

The central problem is **loss of relevant source content**. For the same urllib3 reference page, the native webFetch result was only **761 characters**, mostly introduction text, with the requested Retry details absent. OpenCode's fetch returned **49,428 characters**, including the relevant API documentation. AG1 noticed that its extracts were sparse, then repeatedly tried alternative fragments/URLs and shell parsing. More tools did not resolve this within the cap.

[Normal-toolset native messages](ag1-default-web/raw-messages.json), [result](ag1-default-web/result.json), [observations](observations-summary.json).

### OpenCode

OpenCode made three searches and retrieved substantive search results. Its 624-word visible response used better direct citations and kept the idempotency key stable. It correctly described the documented backoff default.

It nevertheless **claimed to have read all three sources after only two webfetch calls**. The Python domain appeared in search results, not an actual page fetch. Its pseudocode also failed the hard deadline requirement: there was no deadline check at the top of every retry, a fixed request timeout could exceed the remaining budget, and zero/past Retry-After values took an incorrect break path. No production-ready retry policy passed this task.

[Answer](opencode-web/answer.md), [tool events](opencode-web/events.jsonl), [result](opencode-web/result.json).

For the factual review, the current [urllib3 Retry reference](https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html#urllib3.util.Retry) documents the allowed-method set, zero default backoff, and Retry-After behavior. [MDN Retry-After](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Retry-After) documents its two forms; [Python time.monotonic](https://docs.python.org/3/library/time.html#time.monotonic) documents elapsed-time clock behavior. The report evaluates the model outputs; their preserved pseudocode is not an endorsed implementation.

## A separate AG1 presentation defect

Native webFetch HTTP failures were not marked as failed tool results. The first web run had two such errors; the normal-toolset run had three. Their content said `Fetch Error`, but `isError` was false/absent, so the UI could show **“Read webpage” and a source badge** for a failed fetch.

This is an observed native result/classification problem, not an icon-download failure. `product/crates/ag1-platform/src/web_tools.rs` returns `details.error` from `fetch_error` without setting the tool error flag, while the source projection trusts the existing error flag. It needs a regression and a fix before those rows can be considered truthful in all tested states.

## Coding evidence

Both harnesses received byte-identical Python/SQLite projects, the same specification and prompt, and the same unmodified public tests. The task covered atomic batches, durable idempotency, conflicting IDs, input validation, negative-stock prevention, ordering, reopening/connection visibility, and CLI output/exit behavior.

The held-out suite has 18 independent test methods with additional malformed-input subcases. Its definition was hashed before final scoring and remained unchanged. [Scorer](score_code.py), [definition hashes](test-definition-hashes.json), [seed manifest](seed-manifest.json). The original broken seed failed many cases; it was not a vacuous acceptance suite.

- **AG1:** [score and failures](ag1-coding/score.json), [patch](ag1-coding/changes.patch), [raw messages](ag1-coding/raw-messages.json). It diagnosed double-counting late in the run, but the corrective edit was still pending at shutdown. Its recorded files failed the API and CLI negative-stock/atomicity checks. Eight earlier automatic fixture approvals were accepted; their receipts are retained.
- **OpenCode:** [score and failure](opencode-coding/score.json), [patch](opencode-coding/changes.patch), [answer](opencode-coding/answer.md). Its own five supplied tests and additional checks passed, but the independent suite found that unknown event keys were accepted.

The planned dry-run follow-up depended on a completed initial repair. Since AG1 did not finish, that follow-up was omitted from **both** scored adapters. No continuation result is claimed.

## Conditions, fairness and retained setup errors

- RTX 4080, one GPU. Both used **Qwen3.6-35B-A3B-UD-Q4_K_XL**, the same captured llama.cpp binary, GGUF, exact launch arguments, offloading and runtime environment. No model profile, weight or backend was changed.
- Context **90,000**, output allowance **65,536**, thinking enabled. A loopback proxy normalized OpenCode's sampling request to AG1's existing parameters: temperature 0.7, top-p 0.95, top-k 20 and the other recorded native defaults. It did not change messages or tool definitions. [Configuration/normalization record](opencode-isolation.json), [request metadata](opencode-requests.jsonl).
- OpenCode used an isolated configuration and data directories, its normal build agent/tools, `--pure`, and native Exa search. AG1 used the native desktop with isolated projects/state/authority. Both could modify only owned fixture locations. Toolsets, system prompts and search providers are intentionally reported, not represented as identical.
- The [OpenCode local-provider configuration](https://opencode.ai/docs/providers/#llamacpp) and [documented search enablement](https://opencode.ai/docs/tools/#websearch) informed setup. The user's installed OpenCode binary and configuration were not replaced.
- Stages ran sequentially. AG1's runtimes stopped before OpenCode started; OpenCode's server exited successfully before the additional AG1 run. All owned processes were stopped afterward.
- **Timing is end-to-end diagnostic timing, not a pure inference benchmark.** AG1 first-stage counters include native model loading; OpenCode's server was loaded beforehand (7.24 seconds, recorded separately). AG1 coding hit its 360-second observation cap, then spent about 22 seconds in cleanup because the first driver used an incomplete cancel command and an outdated button selector. The table scores it as incomplete and reports its partial files, not a successful 382-second solution.
- The first attempted coding stage failed to select the new native workspace before submission. That setup run is excluded. A fresh coding run explicitly verified native workspace identity before prompting. Its failed setup transcript and the cancellation issue are retained in [protocol deviations](protocol-deviations.json).
- This is a small exploratory comparison, with one scored coding attempt per harness and an explicitly disclosed additional AG1 research diagnostic. It cannot establish statistical superiority or isolate model randomness from harness effects.

## Completion and next work

No production source change or installation was made during this comparison. The installed release remains `/opt/ag1/releases/99654038ec774defb3dffe5b3682832e`. Protected source/model/runtime hashes, the OpenCode binary and test definitions were verified unchanged. Firefox PID 24879 remained running, all owned app/display/model processes stopped, and the GPU had no compute process left. [Final verification](final-verification.json).

The next useful AG1 fixes are: preserve or expose relevant/full documentation sections; make webFetch semantic failures propagate as errors; distinguish official fallback suggestions from search-engine hits; and improve recovery when repeated tools yield no new evidence. Keep the existing icon and thinking-placement behavior, which held up in these sampled native runs. The broader chat/artifact closeout and endurance programme remain separate.
