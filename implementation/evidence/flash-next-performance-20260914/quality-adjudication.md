# Quality adjudication notes

The initial `tool` workload asks for a JSON call object but does not explicitly specify whether its tool-name key is `name` or `tool`. The original model chose `tool`, with the correct read_file operation/path/line range. The automated exact-object comparison against a `name` key is therefore an overly strict fixture, not evidence of a model failure. Raw scores/outputs are retained. Actual OpenAI-compatible function-call tests with a complete function schema are required separately before recommendation.

The timed prose/review/reasoning screens cap generation at256 tokens for consistent throughput sampling. These are not full instruction-adherence evaluations of the requested500-word answer. Complete-answer correctness and actual AG1 runs follow candidate selection.

PLE table hashes have no speculative acceptance rate. Only separately enabled runtime drafting can supply that metric. `ngram-mod` retains a shared dictionary across requests, even when prompt KV caching is disabled; repeated identical answers must not be mistaken for independent fresh-answer acceleration. Speculative candidate discovery will use separate process starts for independent repeats and explicitly label cache/history state.

Original long-context retrieval returned all12 requested field values correctly in the first21,308-token test, but wrapped the JSON in Markdown fences despite the plain-JSON instruction. The repetitive source copy likewise changed the requested values correctly but added code fences. These are baseline formatting/instruction failures, distinct from factual retrieval or edit-content failures. Exact ModelLoadControl file editing passed without wrappers in all three repeats.


## Strict byte audit correction

The runner's historical `quality_pass` compares edited source after `.strip()`. A separate byte audit found that all55 completed exact-edit responses through the fourth-screen checkpoint have the requested content but omit the original final newline. They therefore pass content equivalence but fail literal byte equality. Earlier progress wording that called these results "byte-correct" or "exact" was too strong. See exact-edit-byte-audit.json. Raw response files are preserved.

Future runner records retain the old field for comparability and add quality_content_pass and quality_exact_bytes_pass separately. Final reporting must make this distinction. The real AG1 file-edit validation will check actual on-disk file bytes, including the final newline; a chat response containing source is not itself proof of a correctly written file.
