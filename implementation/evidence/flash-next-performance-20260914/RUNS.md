# Real-model experiment catalog

Generated 2026-09-15 00:07:49 UTC. Investigation closed; see REPORT.md for conclusions and limitations.

Each row is one workload within one server configuration. Values are medians across the listed requests, not pooled averages across tasks. Initial page-cache misses, expert-cache warm-up and thermal/background effects remain in raw telemetry. This catalog includes discovery and final native runs; REPORT.md distinguishes their roles and gives the application results and recommendation.

Exact-edit content checks ignore outer whitespace. The separate byte column includes the trailing newline. Native text generation is not an actual AG1 file-write check. Health endpoint latency is recorded in raw results and is not presented as end-to-end AG1 readiness.

A passed process means the runner completed. It does not mean all formatting, factuality, reasoning or integration checks passed. Those results are recorded separately in REPORT.md.

Native decode timing definitions differ: the original excludes the first token (produced by prefill), while the fork includes it in its rate numerator. Common decode recomputes `(predicted_n - 1) * 1000 / predicted_ms` for every build. Native values remain archived. The correction is about 0.39% for 256-token screens and 0.11% for 938-token edits.

| Run / workload | State | Reps | Prefill tok/s | Decode native | Decode common | TTFT s | Response s | Content / bytes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| [baseline-01](runs/baseline-01/result.json) / review | passed | 3 | 302.029 | 27.988 | 27.988 | 20.696 | 29.781 | — / — |
| [baseline-01](runs/baseline-01/result.json) / edit | passed | 3 | 296.558 | 29.017 | 29.017 | 3.393 | 35.679 | 3/3 / 0/3 |
| [baseline-01](runs/baseline-01/result.json) / prose | passed | 3 | 61.123 | 29.190 | 29.190 | 1.211 | 9.946 | — / — |
| [baseline-01](runs/baseline-01/result.json) / tool | passed | 3 | 58.284 | 28.210 | 28.210 | 1.202 | 2.974 | 0/3 / — |
| [baseline-01](runs/baseline-01/result.json) / conversation | passed | 3 | 66.564 | 28.364 | 28.364 | 0.978 | 9.991 | — / — |
| [baseline-01](runs/baseline-01/result.json) / repetitive | passed | 3 | 193.231 | 29.089 | 29.089 | 2.588 | 17.577 | 0/3 / 0/3 |
| [baseline-01](runs/baseline-01/result.json) / reasoning | passed | 3 | 76.287 | 29.070 | 29.070 | 1.443 | 10.189 | — / — |
| [baseline-01](runs/baseline-01/result.json) / long | passed | 3 | 305.463 | 25.973 | 25.973 | 69.762 | 77.978 | 2/3 / — |
| [batch2048](runs/batch2048/result.json) | failed | 0 | — | — | — | — | — | — |
| [batch2048-cpu44](runs/batch2048-cpu44/result.json) / review | passed | 2 | 410.515 | 27.277 | 27.277 | 15.227 | 24.576 | — / — |
| [batch2048-cpu44](runs/batch2048-cpu44/result.json) / prose | passed | 2 | 58.256 | 28.244 | 28.244 | 1.272 | 10.300 | — / — |
| [batch2048-cpu44](runs/batch2048-cpu44/result.json) / edit | passed | 2 | 286.283 | 28.190 | 28.190 | 3.516 | 36.754 | 2/2 / 0/2 |
| [bulkcopy-base-0](runs/bulkcopy-base-0/result.json) / review | passed | 2 | 286.154 | 27.284 | 27.284 | 21.848 | 31.205 | — / — |
| [bulkcopy-base-0](runs/bulkcopy-base-0/result.json) / prose | passed | 2 | 56.791 | 26.051 | 26.051 | 1.304 | 11.094 | — / — |
| [bulkcopy-base-0](runs/bulkcopy-base-0/result.json) / edit | passed | 2 | 265.196 | 24.046 | 24.046 | 3.795 | 42.763 | 2/2 / 0/2 |
| [bulkcopy-base-1](runs/bulkcopy-base-1/result.json) / review | passed | 2 | 301.275 | 27.698 | 27.698 | 20.748 | 29.954 | — / — |
| [bulkcopy-base-1](runs/bulkcopy-base-1/result.json) / prose | passed | 2 | 60.714 | 28.817 | 28.817 | 1.220 | 10.070 | — / — |
| [bulkcopy-base-1](runs/bulkcopy-base-1/result.json) / edit | passed | 2 | 299.709 | 28.844 | 28.844 | 3.358 | 35.844 | 2/2 / 0/2 |
| [bulkcopy-bulk-0](runs/bulkcopy-bulk-0/result.json) / review | passed | 2 | 252.031 | 27.243 | 27.243 | 24.819 | 34.185 | — / — |
| [bulkcopy-bulk-0](runs/bulkcopy-bulk-0/result.json) / prose | passed | 2 | 60.067 | 28.675 | 28.675 | 1.233 | 10.126 | — / — |
| [bulkcopy-bulk-0](runs/bulkcopy-bulk-0/result.json) / edit | passed | 2 | 254.854 | 28.900 | 28.900 | 3.948 | 36.371 | 2/2 / 0/2 |
| [bulkcopy-bulk-1](runs/bulkcopy-bulk-1/result.json) / review | passed | 2 | 258.002 | 27.260 | 27.260 | 24.227 | 33.582 | — / — |
| [bulkcopy-bulk-1](runs/bulkcopy-bulk-1/result.json) / prose | passed | 2 | 59.113 | 28.221 | 28.221 | 1.253 | 10.289 | — / — |
| [bulkcopy-bulk-1](runs/bulkcopy-bulk-1/result.json) / edit | passed | 2 | 258.039 | 28.128 | 28.128 | 3.900 | 37.212 | 2/2 / 0/2 |
| [cold-ple-0-original-native](runs/cold-ple-0-original-native/result.json) / review | passed | 1 | 286.560 | 28.281 | 28.281 | 21.813 | 30.830 | — / — |
| [cold-ple-0-original-ple-parallel](runs/cold-ple-0-original-ple-parallel/result.json) / review | passed | 1 | 286.448 | 27.941 | 27.941 | 21.821 | 30.948 | — / — |
| [cold-ple-1-original-native](runs/cold-ple-1-original-native/result.json) / review | passed | 1 | 287.023 | 28.081 | 28.081 | 21.778 | 30.859 | — / — |
| [cold-ple-1-original-ple-parallel](runs/cold-ple-1-original-ple-parallel/result.json) / review | passed | 1 | 285.847 | 28.012 | 28.012 | 21.867 | 30.971 | — / — |
| [cold-ple-2-original-native](runs/cold-ple-2-original-native/result.json) / review | passed | 1 | 286.521 | 28.077 | 28.077 | 21.816 | 30.899 | — / — |
| [cold-ple-2-original-ple-parallel](runs/cold-ple-2-original-ple-parallel/result.json) / review | passed | 1 | 287.167 | 27.907 | 27.907 | 21.767 | 30.905 | — / — |
| [cow-bulkcopy-base-0](runs/cow-bulkcopy-base-0/result.json) / review | passed | 2 | 406.405 | 27.338 | 27.338 | 15.402 | 24.732 | — / — |
| [cow-bulkcopy-base-0](runs/cow-bulkcopy-base-0/result.json) / prose | passed | 2 | 88.879 | 28.441 | 28.441 | 0.834 | 9.807 | — / — |
| [cow-bulkcopy-base-0](runs/cow-bulkcopy-base-0/result.json) / edit | passed | 2 | 404.840 | 27.350 | 27.350 | 2.486 | 36.772 | 2/2 / 0/2 |
| [cow-bulkcopy-base-1](runs/cow-bulkcopy-base-1/result.json) / review | passed | 2 | 410.297 | 27.903 | 27.903 | 15.264 | 24.407 | — / — |
| [cow-bulkcopy-base-1](runs/cow-bulkcopy-base-1/result.json) / prose | passed | 2 | 91.033 | 29.112 | 29.112 | 0.814 | 9.573 | — / — |
| [cow-bulkcopy-base-1](runs/cow-bulkcopy-base-1/result.json) / edit | passed | 2 | 408.061 | 29.171 | 29.171 | 2.466 | 34.588 | 2/2 / 0/2 |
| [cow-bulkcopy-bulk-0](runs/cow-bulkcopy-bulk-0/result.json) / review | passed | 2 | 407.452 | 27.100 | 27.100 | 15.366 | 24.778 | — / — |
| [cow-bulkcopy-bulk-0](runs/cow-bulkcopy-bulk-0/result.json) / prose | passed | 2 | 84.512 | 27.746 | 27.746 | 0.878 | 10.069 | — / — |
| [cow-bulkcopy-bulk-0](runs/cow-bulkcopy-bulk-0/result.json) / edit | passed | 2 | 405.810 | 27.963 | 27.963 | 2.480 | 35.989 | 2/2 / 0/2 |
| [cow-bulkcopy-bulk-1](runs/cow-bulkcopy-bulk-1/result.json) / review | passed | 2 | 407.654 | 27.247 | 27.247 | 15.359 | 24.722 | — / — |
| [cow-bulkcopy-bulk-1](runs/cow-bulkcopy-bulk-1/result.json) / prose | passed | 2 | 71.361 | 27.500 | 27.500 | 1.040 | 10.313 | — / — |
| [cow-bulkcopy-bulk-1](runs/cow-bulkcopy-bulk-1/result.json) / edit | passed | 2 | 406.780 | 29.041 | 29.041 | 2.474 | 34.740 | 2/2 / 0/2 |
| [cow-prefault-0-0](runs/cow-prefault-0-0/result.json) / prose | passed | 1 | 67.295 | 23.270 | 23.270 | 1.101 | 3.808 | — / — |
| [cow-prefault-0-1](runs/cow-prefault-0-1/result.json) / prose | passed | 1 | 72.791 | 27.783 | 27.783 | 1.019 | 3.286 | — / — |
| [cow-prefault-0-2](runs/cow-prefault-0-2/result.json) / prose | passed | 1 | 70.764 | 27.581 | 27.581 | 1.048 | 3.332 | — / — |
| [cow-prefault-12-0](runs/cow-prefault-12-0/result.json) / prose | passed | 1 | 75.541 | 28.068 | 28.068 | 0.981 | 3.226 | — / — |
| [cow-prefault-12-1](runs/cow-prefault-12-1/result.json) / prose | passed | 1 | 75.289 | 28.055 | 28.055 | 0.985 | 3.230 | — / — |
| [cow-prefault-12-2](runs/cow-prefault-12-2/result.json) / prose | passed | 1 | 68.585 | 28.007 | 28.007 | 1.082 | 3.330 | — / — |
| [cow-prefault-6-0](runs/cow-prefault-6-0/result.json) / prose | passed | 1 | 74.493 | 27.906 | 27.906 | 0.999 | 3.257 | — / — |
| [cow-prefault-6-1](runs/cow-prefault-6-1/result.json) / prose | passed | 1 | 56.917 | 23.278 | 23.278 | 1.302 | 4.008 | — / — |
| [cow-prefault-6-2](runs/cow-prefault-6-2/result.json) / prose | passed | 1 | 75.247 | 27.863 | 27.863 | 0.986 | 3.246 | — / — |
| [cpu-moe41](runs/cpu-moe41/result.json) / review | passed | 2 | 305.962 | 28.313 | 28.313 | 20.430 | 29.437 | — / — |
| [cpu-moe41](runs/cpu-moe41/result.json) / prose | passed | 2 | 62.081 | 29.596 | 29.596 | 1.193 | 9.809 | — / — |
| [cpu-moe41](runs/cpu-moe41/result.json) / edit | passed | 2 | 298.248 | 29.317 | 29.317 | 3.374 | 35.336 | 2/2 / 0/2 |
| [cuda-host-controlled-load-0](runs/cuda-host-controlled-load-0/result.json) / prose | passed | 1 | 52.782 | 24.554 | 24.554 | 1.407 | 3.973 | — / — |
| [cuda-host-controlled-load-1](runs/cuda-host-controlled-load-1/result.json) / prose | passed | 1 | 68.704 | 28.495 | 28.495 | 1.078 | 3.289 | — / — |
| [diagnostic-original-cupti-perf](runs/diagnostic-original-cupti-perf/result.json) / review | instrumented | 1 | 282.008 | 20.512 | 20.512 | 22.169 | 31.477 | — / — |
| [diagnostic-original-cupti-perf](runs/diagnostic-original-cupti-perf/result.json) / prose | instrumented | 1 | 50.229 | 21.220 | 21.220 | 1.474 | 10.475 | — / — |
| [final-candidate-0](runs/final-candidate-0/result.json) / review | passed | 1 | 384.371 | 27.914 | 27.914 | 16.269 | 62.698 | — / — |
| [final-candidate-0](runs/final-candidate-0/result.json) / edit | passed | 1 | 422.982 | 56.190 | 56.190 | 2.380 | 19.056 | 1/1 / 0/1 |
| [final-candidate-0](runs/final-candidate-0/result.json) / prose | passed | 1 | 97.820 | 29.324 | 29.324 | 0.758 | 26.164 | — / — |
| [final-candidate-0](runs/final-candidate-0/result.json) / tool | passed | 1 | 93.611 | 28.536 | 28.536 | 0.749 | 2.501 | 0/1 / — |
| [final-candidate-0](runs/final-candidate-0/result.json) / conversation | passed | 1 | 88.437 | 24.429 | 24.429 | 0.737 | 74.380 | — / — |
| [final-candidate-0](runs/final-candidate-0/result.json) / repetitive | passed | 1 | 278.337 | 22.166 | 22.166 | 1.798 | 21.468 | 0/1 / 0/1 |
| [final-candidate-0](runs/final-candidate-0/result.json) / reasoning | passed | 1 | 119.137 | 27.307 | 27.307 | 0.924 | 46.077 | — / — |
| [final-candidate-1](runs/final-candidate-1/result.json) / review | passed | 1 | 388.985 | 25.997 | 25.997 | 16.071 | 64.696 | — / — |
| [final-candidate-1](runs/final-candidate-1/result.json) / edit | passed | 1 | 412.062 | 56.041 | 56.041 | 2.443 | 19.163 | 1/1 / 0/1 |
| [final-candidate-1](runs/final-candidate-1/result.json) / prose | passed | 1 | 95.958 | 29.195 | 29.195 | 0.772 | 26.290 | — / — |
| [final-candidate-1](runs/final-candidate-1/result.json) / tool | passed | 1 | 93.163 | 27.814 | 27.814 | 0.752 | 2.550 | 0/1 / — |
| [final-candidate-1](runs/final-candidate-1/result.json) / conversation | passed | 1 | 88.160 | 28.640 | 28.640 | 0.739 | 63.553 | — / — |
| [final-candidate-1](runs/final-candidate-1/result.json) / repetitive | passed | 1 | 285.805 | 23.772 | 23.772 | 1.751 | 20.092 | 0/1 / 0/1 |
| [final-candidate-1](runs/final-candidate-1/result.json) / reasoning | passed | 1 | 122.295 | 29.341 | 29.341 | 0.900 | 42.924 | — / — |
| [final-config-only-0](runs/final-config-only-0/result.json) / review | passed | 1 | 274.321 | 25.382 | 25.382 | 22.786 | 75.658 | — / — |
| [final-config-only-0](runs/final-config-only-0/result.json) / edit | passed | 1 | 288.643 | 51.230 | 51.230 | 3.487 | 21.777 | 1/1 / 0/1 |
| [final-config-only-0](runs/final-config-only-0/result.json) / prose | passed | 1 | 60.142 | 27.270 | 27.270 | 1.232 | 26.461 | — / — |
| [final-config-only-0](runs/final-config-only-0/result.json) / tool | passed | 1 | 58.844 | 26.506 | 26.506 | 1.190 | 3.077 | 0/1 / — |
| [final-config-only-0](runs/final-config-only-0/result.json) / conversation | passed | 1 | 63.924 | 26.050 | 26.050 | 1.019 | 70.079 | — / — |
| [final-config-only-0](runs/final-config-only-0/result.json) / repetitive | passed | 1 | 186.830 | 21.378 | 21.378 | 2.678 | 23.073 | 0/1 / 0/1 |
| [final-config-only-0](runs/final-config-only-0/result.json) / reasoning | passed | 1 | 72.882 | 26.389 | 26.389 | 1.510 | 61.422 | — / — |
| [final-config-only-1](runs/final-config-only-1/result.json) / review | passed | 1 | 288.135 | 28.373 | 28.373 | 21.694 | 71.072 | — / — |
| [final-config-only-1](runs/final-config-only-1/result.json) / edit | passed | 1 | 304.102 | 54.518 | 54.518 | 3.309 | 20.497 | 1/1 / 0/1 |
| [final-config-only-1](runs/final-config-only-1/result.json) / prose | passed | 1 | 63.197 | 29.337 | 29.337 | 1.172 | 24.624 | — / — |
| [final-config-only-1](runs/final-config-only-1/result.json) / tool | passed | 1 | 61.095 | 28.262 | 28.262 | 1.147 | 2.916 | 0/1 / — |
| [final-config-only-1](runs/final-config-only-1/result.json) / conversation | passed | 1 | 62.599 | 28.531 | 28.531 | 1.040 | 64.095 | — / — |
| [final-config-only-1](runs/final-config-only-1/result.json) / repetitive | passed | 1 | 197.092 | 23.484 | 23.484 | 2.538 | 21.104 | 0/1 / 0/1 |
| [final-config-only-1](runs/final-config-only-1/result.json) / reasoning | passed | 1 | 80.504 | 29.281 | 29.281 | 1.367 | 55.361 | — / — |
| [final-long80k-candidate-0](runs/final-long80k-candidate-0/result.json) / long80k | passed | 1 | 395.381 | 23.318 | 23.318 | 214.647 | 223.313 | 1/1 / — |
| [final-long80k-original-0](runs/final-long80k-original-0/result.json) / long80k | passed | 1 | 281.812 | 20.466 | 20.466 | 301.139 | 311.013 | 1/1 / — |
| [final-original-0](runs/final-original-0/result.json) / review | passed | 1 | 287.753 | 25.372 | 25.372 | 21.723 | 78.714 | — / — |
| [final-original-0](runs/final-original-0/result.json) / edit | passed | 1 | 266.048 | 25.303 | 25.303 | 3.783 | 40.813 | 1/1 / 0/1 |
| [final-original-0](runs/final-original-0/result.json) / prose | passed | 1 | 59.791 | 26.383 | 26.383 | 1.239 | 27.316 | — / — |
| [final-original-0](runs/final-original-0/result.json) / tool | passed | 1 | 50.821 | 24.235 | 24.235 | 1.378 | 3.442 | 0/1 / — |
| [final-original-0](runs/final-original-0/result.json) / conversation | passed | 1 | 61.222 | 26.451 | 26.451 | 1.063 | 69.077 | — / — |
| [final-original-0](runs/final-original-0/result.json) / repetitive | passed | 1 | 190.332 | 24.324 | 24.324 | 2.628 | 20.553 | 0/1 / 0/1 |
| [final-original-0](runs/final-original-0/result.json) / reasoning | passed | 1 | 67.061 | 24.412 | 24.412 | 1.641 | 66.405 | — / — |
| [final-original-1](runs/final-original-1/result.json) / review | passed | 1 | 300.020 | 28.144 | 28.144 | 20.834 | 71.574 | — / — |
| [final-original-1](runs/final-original-1/result.json) / edit | passed | 1 | 296.280 | 29.201 | 29.201 | 3.397 | 35.485 | 1/1 / 0/1 |
| [final-original-1](runs/final-original-1/result.json) / prose | passed | 1 | 61.866 | 29.453 | 29.453 | 1.197 | 24.556 | — / — |
| [final-original-1](runs/final-original-1/result.json) / tool | passed | 1 | 59.293 | 28.565 | 28.565 | 1.181 | 2.932 | 0/1 / — |
| [final-original-1](runs/final-original-1/result.json) / conversation | passed | 1 | 68.189 | 28.544 | 28.544 | 0.955 | 63.981 | — / — |
| [final-original-1](runs/final-original-1/result.json) / repetitive | passed | 1 | 194.610 | 29.188 | 29.188 | 2.571 | 17.509 | 0/1 / 0/1 |
| [final-original-1](runs/final-original-1/result.json) / reasoning | passed | 1 | 78.110 | 29.432 | 29.432 | 1.409 | 55.126 | — / — |
| [long80k-base-0](runs/long80k-base-0/result.json) / long80k | passed | 1 | 283.004 | 20.590 | 20.590 | 299.871 | 309.684 | 1/1 / — |
| [long80k-base-1](runs/long80k-base-1/result.json) / long80k | passed | 1 | 283.482 | 20.650 | 20.650 | 299.365 | 309.150 | 1/1 / — |
| [long80k-candidate-0](runs/long80k-candidate-0/result.json) / long80k | passed | 1 | 286.161 | 23.268 | 23.268 | 296.563 | 305.247 | 1/1 / — |
| [long80k-candidate-1](runs/long80k-candidate-1/result.json) / long80k | passed | 1 | 283.590 | 23.370 | 23.370 | 299.251 | 307.898 | 1/1 / — |
| [long80k-original-0](runs/long80k-original-0/result.json) / long80k | passed | 1 | 277.855 | 20.035 | 20.035 | 305.427 | 315.512 | 1/1 / — |
| [long80k-original-1](runs/long80k-original-1/result.json) / long80k | passed | 1 | 276.058 | 20.013 | 20.013 | 307.417 | 317.514 | 1/1 / — |
| [maskskip-affected-review-base-a](runs/maskskip-affected-review-base-a/result.json) / review | passed | 1 | 296.647 | 26.565 | 26.565 | 21.071 | 23.443 | — / — |
| [maskskip-affected-review-base-b](runs/maskskip-affected-review-base-b/result.json) / review | passed | 1 | 298.243 | 27.575 | 27.575 | 20.958 | 23.243 | — / — |
| [maskskip-affected-review-candidate](runs/maskskip-affected-review-candidate/result.json) / review | passed | 1 | 293.403 | 24.842 | 24.842 | 21.304 | 23.840 | — / — |
| [maskskip-base-real](runs/maskskip-base-real/result.json) / review | passed | 3 | 300.889 | 27.976 | 27.976 | 20.774 | 30.349 | — / — |
| [maskskip-base-real](runs/maskskip-base-real/result.json) / prose | passed | 3 | 58.611 | 29.033 | 29.033 | 1.263 | 10.046 | — / — |
| [maskskip-base-real](runs/maskskip-base-real/result.json) / edit | passed | 3 | 295.971 | 28.899 | 28.899 | 3.400 | 35.901 | 3/3 / 0/3 |
| [maskskip-base-real](runs/maskskip-base-real/result.json) / conversation | passed | 3 | 61.061 | 28.000 | 28.000 | 1.066 | 10.185 | — / — |
| [maskskip-base-real-resume1](runs/maskskip-base-real-resume1/result.json) / review | passed | 3 | 301.655 | 28.172 | 28.172 | 20.721 | 29.764 | — / — |
| [maskskip-base-real-resume1](runs/maskskip-base-real-resume1/result.json) / prose | passed | 3 | 60.311 | 28.762 | 28.762 | 1.228 | 10.086 | — / — |
| [maskskip-base-real-resume1](runs/maskskip-base-real-resume1/result.json) / edit | passed | 3 | 294.660 | 28.731 | 28.731 | 3.415 | 36.013 | 3/3 / 0/3 |
| [maskskip-base-real-resume1](runs/maskskip-base-real-resume1/result.json) / conversation | passed | 3 | 67.945 | 29.259 | 29.259 | 0.959 | 9.674 | — / — |
| [maskskip-candidate-real-resume1](runs/maskskip-candidate-real-resume1/result.json) / review | passed | 3 | 302.268 | 28.064 | 28.064 | 20.679 | 29.905 | — / — |
| [maskskip-candidate-real-resume1](runs/maskskip-candidate-real-resume1/result.json) / prose | passed | 3 | 60.792 | 28.989 | 28.989 | 1.218 | 10.010 | — / — |
| [maskskip-candidate-real-resume1](runs/maskskip-candidate-real-resume1/result.json) / edit | passed | 3 | 298.483 | 28.983 | 28.983 | 3.372 | 35.701 | 3/3 / 0/3 |
| [maskskip-candidate-real-resume1](runs/maskskip-candidate-real-resume1/result.json) / conversation | passed | 3 | 67.201 | 28.369 | 28.369 | 0.969 | 9.950 | — / — |
| [mtp-control-cpu46](runs/mtp-control-cpu46/result.json) / prose | passed | 2 | 53.535 | 27.198 | 27.198 | 1.385 | 10.760 | — / — |
| [mtp-control-cpu46](runs/mtp-control-cpu46/result.json) / edit | passed | 2 | 274.110 | 27.155 | 27.155 | 3.672 | 38.178 | 2/2 / 0/2 |
| [mtp-control-cpu46](runs/mtp-control-cpu46/result.json) / reasoning | passed | 2 | 70.199 | 27.205 | 27.205 | 1.568 | 10.941 | — / — |
| [mtp-q8-cpu46-max2](runs/mtp-q8-cpu46-max2/result.json) / prose | passed | 2 | 46.960 | 26.713 | 26.713 | 1.618 | 11.166 | — / — |
| [mtp-q8-cpu46-max2](runs/mtp-q8-cpu46-max2/result.json) / edit | passed | 2 | 256.901 | 32.910 | 32.910 | 3.934 | 32.407 | 2/2 / 0/2 |
| [mtp-q8-cpu46-max2](runs/mtp-q8-cpu46-max2/result.json) / reasoning | passed | 2 | 71.313 | 25.859 | 25.859 | 1.544 | 11.409 | — / — |
| [mtp-q8-cpu46-max4](runs/mtp-q8-cpu46-max4/result.json) / prose | passed | 2 | 49.081 | 22.145 | 22.145 | 1.525 | 13.045 | — / — |
| [mtp-q8-cpu46-max4](runs/mtp-q8-cpu46-max4/result.json) / edit | passed | 2 | 269.643 | 37.087 | 37.087 | 3.732 | 29.002 | 2/2 / 0/2 |
| [mtp-q8-cpu46-max4](runs/mtp-q8-cpu46-max4/result.json) / reasoning | passed | 2 | 69.982 | 24.696 | 24.696 | 1.573 | 11.899 | — / — |
| [native-original](runs/native-original/result.json) / review | passed | 2 | 301.268 | 28.036 | 28.036 | 20.749 | 29.845 | — / — |
| [native-original](runs/native-original/result.json) / prose | passed | 2 | 60.685 | 29.146 | 29.146 | 1.220 | 9.970 | — / — |
| [native-original](runs/native-original/result.json) / edit | passed | 2 | 296.190 | 29.069 | 29.069 | 3.398 | 35.632 | 2/2 / 0/2 |
| [new-ada](runs/new-ada/result.json) / review | passed | 2 | 297.001 | 27.672 | 27.672 | 21.052 | 30.267 | — / — |
| [new-ada](runs/new-ada/result.json) / prose | passed | 2 | 60.198 | 28.356 | 28.356 | 1.230 | 10.223 | — / — |
| [new-ada](runs/new-ada/result.json) / edit | passed | 2 | 294.019 | 27.562 | 27.562 | 3.423 | 37.427 | 2/2 / 0/2 |
| [new-cow-b2048-cpu44](runs/new-cow-b2048-cpu44/result.json) / review | passed | 2 | 502.904 | 22.548 | 22.548 | 12.459 | 23.781 | — / — |
| [new-cow-b2048-cpu44](runs/new-cow-b2048-cpu44/result.json) / prose | passed | 2 | 72.479 | 22.855 | 22.855 | 1.023 | 12.180 | — / — |
| [new-cow-b2048-cpu44](runs/new-cow-b2048-cpu44/result.json) / edit | passed | 2 | 365.803 | 24.156 | 24.156 | 2.752 | 41.734 | 2/2 / 0/2 |
| [new-cow-pinned](runs/new-cow-pinned/result.json) / review | passed | 2 | 409.290 | 28.012 | 28.012 | 15.305 | 24.410 | — / — |
| [new-cow-pinned](runs/new-cow-pinned/result.json) / prose | passed | 2 | 90.741 | 29.172 | 29.172 | 0.817 | 9.558 | — / — |
| [new-cow-pinned](runs/new-cow-pinned/result.json) / edit | passed | 2 | 408.246 | 28.863 | 28.863 | 2.465 | 34.932 | 2/2 / 0/2 |
| [new-cpu-flat](runs/new-cpu-flat/result.json) | excluded | 0 | — | — | — | — | — | — |
| [new-cpu-flat-verified](runs/new-cpu-flat-verified/result.json) / review | passed | 2 | 298.311 | 28.705 | 28.705 | 20.957 | 29.841 | — / — |
| [new-cpu-flat-verified](runs/new-cpu-flat-verified/result.json) / prose | passed | 2 | 60.195 | 29.792 | 29.792 | 1.230 | 9.790 | — / — |
| [new-cpu-flat-verified](runs/new-cpu-flat-verified/result.json) / edit | passed | 2 | 292.370 | 29.705 | 29.705 | 3.442 | 34.986 | 2/2 / 0/2 |
| [new-direct-registered](runs/new-direct-registered/result.json) / review | passed | 2 | 300.514 | 28.327 | 28.327 | 20.800 | 29.802 | — / — |
| [new-direct-registered](runs/new-direct-registered/result.json) / prose | passed | 2 | 60.597 | 29.235 | 29.235 | 1.222 | 9.945 | — / — |
| [new-direct-registered](runs/new-direct-registered/result.json) / edit | passed | 2 | 296.006 | 29.328 | 29.328 | 3.400 | 35.349 | 2/2 / 0/2 |
| [new-graph-opt](runs/new-graph-opt/result.json) / review | passed | 2 | 297.996 | 28.172 | 28.172 | 20.976 | 30.028 | — / — |
| [new-graph-opt](runs/new-graph-opt/result.json) / prose | passed | 2 | 59.902 | 29.161 | 29.161 | 1.236 | 9.981 | — / — |
| [new-graph-opt](runs/new-graph-opt/result.json) / edit | passed | 2 | 293.624 | 29.131 | 29.131 | 3.428 | 35.593 | 2/2 / 0/2 |
| [new-local-control](runs/new-local-control/result.json) / review | passed | 1 | 298.609 | 28.051 | 28.051 | 20.933 | 30.024 | — / — |
| [new-local-control](runs/new-local-control/result.json) / prose | passed | 1 | 60.201 | 29.080 | 29.080 | 1.230 | 9.999 | — / — |
| [new-local-control](runs/new-local-control/result.json) / edit | passed | 1 | 297.329 | 29.316 | 29.316 | 3.384 | 35.347 | 1/1 / 0/1 |
| [new-no-graphs](runs/new-no-graphs/result.json) / review | passed | 2 | 282.154 | 23.232 | 23.232 | 22.163 | 33.141 | — / — |
| [new-no-graphs](runs/new-no-graphs/result.json) / prose | passed | 2 | 58.184 | 24.591 | 24.591 | 1.274 | 11.645 | — / — |
| [new-no-graphs](runs/new-no-graphs/result.json) / edit | passed | 2 | 275.633 | 24.853 | 24.853 | 3.661 | 41.364 | 2/2 / 0/2 |
| [new-noomp-poll0](runs/new-noomp-poll0/result.json) / review | passed | 2 | 302.170 | 27.075 | 27.075 | 20.689 | 30.107 | — / — |
| [new-noomp-poll0](runs/new-noomp-poll0/result.json) / prose | passed | 2 | 63.718 | 27.797 | 27.797 | 1.162 | 10.336 | — / — |
| [new-noomp-poll0](runs/new-noomp-poll0/result.json) / edit | passed | 2 | 302.413 | 27.869 | 27.869 | 3.329 | 36.950 | 2/2 / 0/2 |
| [new-noomp-registered](runs/new-noomp-registered/result.json) / review | passed | 2 | 300.895 | 28.374 | 28.374 | 20.775 | 29.762 | — / — |
| [new-noomp-registered](runs/new-noomp-registered/result.json) / prose | passed | 2 | 63.475 | 29.539 | 29.539 | 1.167 | 9.799 | — / — |
| [new-noomp-registered](runs/new-noomp-registered/result.json) / edit | passed | 2 | 304.476 | 29.501 | 29.501 | 3.306 | 35.067 | 2/2 / 0/2 |
| [new-omp-passive](runs/new-omp-passive/result.json) / review | passed | 2 | 301.133 | 25.819 | 25.819 | 20.758 | 30.635 | — / — |
| [new-omp-passive](runs/new-omp-passive/result.json) / prose | passed | 2 | 63.050 | 26.439 | 26.439 | 1.174 | 10.819 | — / — |
| [new-omp-passive](runs/new-omp-passive/result.json) / edit | passed | 2 | 300.865 | 26.173 | 26.173 | 3.345 | 39.146 | 2/2 / 0/2 |
| [new-pool-efficient64-stage](runs/new-pool-efficient64-stage/result.json) / review | passed | 2 | 314.008 | 25.896 | 25.896 | 19.910 | 29.757 | — / — |
| [new-pool-efficient64-stage](runs/new-pool-efficient64-stage/result.json) / prose | passed | 2 | 70.553 | 27.454 | 27.454 | 1.050 | 10.338 | — / — |
| [new-pool-efficient64-stage](runs/new-pool-efficient64-stage/result.json) / edit | passed | 2 | 311.825 | 26.244 | 26.244 | 3.227 | 38.931 | 2/2 / 0/2 |
| [new-pool-efficient80-stage](runs/new-pool-efficient80-stage/result.json) / review | passed | 2 | 315.583 | 27.687 | 27.687 | 19.808 | 29.018 | — / — |
| [new-pool-efficient80-stage](runs/new-pool-efficient80-stage/result.json) / prose | passed | 2 | 72.158 | 30.087 | 30.087 | 1.026 | 9.502 | — / — |
| [new-pool-efficient80-stage](runs/new-pool-efficient80-stage/result.json) / edit | passed | 2 | 310.677 | 28.263 | 28.263 | 3.240 | 36.392 | 2/2 / 0/2 |
| [new-pool32-cpu48](runs/new-pool32-cpu48/result.json) / review | passed | 2 | 274.873 | 18.081 | 18.081 | 22.741 | 36.845 | — / — |
| [new-pool32-cpu48](runs/new-pool32-cpu48/result.json) / prose | passed | 2 | 55.777 | 18.840 | 18.840 | 1.328 | 14.863 | — / — |
| [new-pool32-cpu48](runs/new-pool32-cpu48/result.json) / edit | passed | 2 | 275.816 | 18.038 | 18.038 | 3.649 | 55.595 | 2/2 / 0/2 |
| [new-pool64-pinned](runs/new-pool64-pinned/result.json) / review | passed | 2 | 377.491 | 27.650 | 27.650 | 16.585 | 25.810 | — / — |
| [new-pool64-pinned](runs/new-pool64-pinned/result.json) / prose | passed | 2 | 84.710 | 28.843 | 28.843 | 0.875 | 9.718 | — / — |
| [new-pool64-pinned](runs/new-pool64-pinned/result.json) / edit | passed | 2 | 391.697 | 29.468 | 29.468 | 2.571 | 34.385 | 2/2 / 0/2 |
| [new-pool64-stage](runs/new-pool64-stage/result.json) / review | passed | 2 | 305.006 | 24.727 | 24.727 | 20.494 | 30.815 | — / — |
| [new-pool64-stage](runs/new-pool64-stage/result.json) / prose | passed | 2 | 67.120 | 25.463 | 25.463 | 1.103 | 11.118 | — / — |
| [new-pool64-stage](runs/new-pool64-stage/result.json) / edit | passed | 2 | 292.764 | 24.906 | 24.906 | 3.439 | 41.061 | 2/2 / 0/2 |
| [new-pool80-stage](runs/new-pool80-stage/result.json) / review | passed | 2 | 303.399 | 25.724 | 25.724 | 20.603 | 30.525 | — / — |
| [new-pool80-stage](runs/new-pool80-stage/result.json) / prose | passed | 2 | 64.861 | 27.353 | 27.353 | 1.142 | 10.464 | — / — |
| [new-pool80-stage](runs/new-pool80-stage/result.json) / edit | passed | 2 | 279.928 | 26.457 | 26.457 | 3.595 | 39.020 | 2/2 / 0/2 |
| [new-registered](runs/new-registered/result.json) / review | passed | 2 | 302.011 | 28.262 | 28.262 | 20.697 | 29.720 | — / — |
| [new-registered](runs/new-registered/result.json) / prose | passed | 2 | 59.826 | 29.311 | 29.311 | 1.238 | 9.938 | — / — |
| [new-registered](runs/new-registered/result.json) / edit | passed | 2 | 287.610 | 29.166 | 29.166 | 3.503 | 35.629 | 2/2 / 0/2 |
| [new-rows2](runs/new-rows2/result.json) / review | passed | 2 | 301.010 | 28.261 | 28.261 | 20.767 | 29.791 | — / — |
| [new-rows2](runs/new-rows2/result.json) / prose | passed | 2 | 60.649 | 29.257 | 29.257 | 1.221 | 9.937 | — / — |
| [new-rows2](runs/new-rows2/result.json) / edit | passed | 2 | 297.378 | 29.289 | 29.289 | 3.384 | 35.376 | 2/2 / 0/2 |
| [new-stage-b1536-cpu42](runs/new-stage-b1536-cpu42/result.json) / review | passed | 2 | 356.853 | 24.759 | 24.759 | 17.525 | 27.825 | — / — |
| [new-stage-b1536-cpu42](runs/new-stage-b1536-cpu42/result.json) / prose | passed | 2 | 66.514 | 25.943 | 25.943 | 1.115 | 10.947 | — / — |
| [new-stage-b1536-cpu42](runs/new-stage-b1536-cpu42/result.json) / edit | passed | 2 | 281.160 | 27.453 | 27.453 | 3.580 | 37.815 | 2/2 / 0/2 |
| [new-stage-b2048-cpu44](runs/new-stage-b2048-cpu44/result.json) / review | passed | 2 | 417.085 | 27.486 | 27.486 | 14.989 | 24.266 | — / — |
| [new-stage-b2048-cpu44](runs/new-stage-b2048-cpu44/result.json) / prose | passed | 2 | 66.339 | 27.629 | 27.629 | 1.120 | 10.351 | — / — |
| [new-stage-b2048-cpu44](runs/new-stage-b2048-cpu44/result.json) / edit | passed | 2 | 300.139 | 26.107 | 26.107 | 3.363 | 39.283 | 2/2 / 0/2 |
| [new-stage-b4096-cpu48](runs/new-stage-b4096-cpu48/result.json) / review | passed | 2 | 469.409 | 20.861 | 20.861 | 13.326 | 25.550 | — / — |
| [new-stage-b4096-cpu48](runs/new-stage-b4096-cpu48/result.json) / prose | passed | 2 | 53.346 | 21.049 | 21.049 | 1.388 | 13.503 | — / — |
| [new-stage-b4096-cpu48](runs/new-stage-b4096-cpu48/result.json) / edit | passed | 2 | 283.563 | 25.994 | 25.994 | 3.569 | 39.624 | 2/2 / 0/2 |
| [new-stage-original-placement](runs/new-stage-original-placement/result.json) / review | passed | 2 | 307.590 | 25.701 | 25.701 | 20.324 | 30.246 | — / — |
| [new-stage-original-placement](runs/new-stage-original-placement/result.json) / prose | passed | 2 | 72.904 | 28.465 | 28.465 | 1.016 | 9.977 | — / — |
| [new-stage-original-placement](runs/new-stage-original-placement/result.json) / edit | passed | 2 | 308.714 | 28.380 | 28.380 | 3.263 | 36.281 | 2/2 / 0/2 |
| [newer-pinned](runs/newer-pinned/result.json) / review | passed | 2 | 409.072 | 28.095 | 28.095 | 15.328 | 24.405 | — / — |
| [newer-pinned](runs/newer-pinned/result.json) / prose | passed | 2 | 91.059 | 29.048 | 29.048 | 0.813 | 9.592 | — / — |
| [newer-pinned](runs/newer-pinned/result.json) / edit | passed | 2 | 406.706 | 29.107 | 29.107 | 2.476 | 34.668 | 2/2 / 0/2 |
| [newer-stock](runs/newer-stock/result.json) / review | passed | 2 | 299.188 | 28.222 | 28.222 | 20.892 | 29.928 | — / — |
| [newer-stock](runs/newer-stock/result.json) / prose | passed | 2 | 60.431 | 29.172 | 29.172 | 1.225 | 9.967 | — / — |
| [newer-stock](runs/newer-stock/result.json) / edit | passed | 2 | 292.285 | 29.192 | 29.192 | 3.443 | 35.541 | 2/2 / 0/2 |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/result.json) / edit | passed | 1 | 277.456 | 50.333 | 50.333 | 3.627 | 22.243 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/result.json) / repetitive | passed | 1 | 186.251 | 25.991 | 25.991 | 2.686 | 19.461 | 0/1 / 0/1 |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/result.json) / copy_source | passed | 1 | 285.193 | 53.550 | 53.550 | 3.462 | 20.960 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/result.json) / copy_repetitive | passed | 1 | 183.008 | 26.087 | 26.087 | 2.695 | 19.677 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/result.json) / prose | passed | 1 | 59.014 | 26.228 | 26.228 | 1.255 | 10.978 | — / — |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/result.json) / edit | passed | 1 | 274.245 | 51.918 | 51.918 | 3.669 | 21.718 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/result.json) / repetitive | passed | 1 | 194.311 | 22.545 | 22.545 | 2.574 | 21.913 | 0/1 / 0/1 |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/result.json) / copy_source | passed | 1 | 271.280 | 46.279 | 46.279 | 3.639 | 23.887 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/result.json) / copy_repetitive | passed | 1 | 166.461 | 28.136 | 28.136 | 2.964 | 18.709 | 1/1 / 0/1 |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/result.json) / prose | passed | 1 | 63.102 | 27.860 | 27.860 | 1.173 | 10.327 | — / — |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/result.json) / edit | passed | 1 | 276.396 | 51.035 | 51.035 | 3.641 | 22.001 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/result.json) / repetitive | passed | 1 | 186.148 | 23.563 | 23.563 | 2.687 | 21.191 | 0/1 / 0/1 |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/result.json) / copy_source | passed | 1 | 300.658 | 59.444 | 59.444 | 3.284 | 19.047 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/result.json) / copy_repetitive | passed | 1 | 179.872 | 34.203 | 34.203 | 2.742 | 15.695 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/result.json) / prose | passed | 1 | 63.798 | 28.075 | 28.075 | 1.161 | 10.244 | — / — |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/result.json) / edit | passed | 1 | 259.991 | 48.323 | 48.323 | 3.870 | 23.261 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/result.json) / repetitive | passed | 1 | 196.186 | 19.175 | 19.175 | 2.550 | 25.288 | 0/1 / 0/1 |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/result.json) / copy_source | passed | 1 | 303.275 | 61.418 | 61.418 | 3.256 | 18.512 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/result.json) / copy_repetitive | passed | 1 | 169.118 | 36.654 | 36.654 | 2.916 | 15.003 | 1/1 / 0/1 |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/result.json) / prose | passed | 1 | 62.548 | 27.413 | 27.413 | 1.184 | 10.486 | — / — |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/result.json) / edit | passed | 1 | 275.567 | 26.723 | 26.723 | 3.652 | 38.715 | 1/1 / 0/1 |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/result.json) / repetitive | passed | 1 | 189.422 | 26.993 | 26.993 | 2.641 | 18.793 | 0/1 / 0/1 |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/result.json) / copy_source | passed | 1 | 292.791 | 26.959 | 26.959 | 3.372 | 38.129 | 1/1 / 0/1 |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/result.json) / copy_repetitive | passed | 1 | 184.782 | 27.087 | 27.087 | 2.669 | 19.024 | 1/1 / 0/1 |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/result.json) / prose | passed | 1 | 60.931 | 27.063 | 27.063 | 1.215 | 10.638 | — / — |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/result.json) / edit | passed | 1 | 278.089 | 28.936 | 28.936 | 3.619 | 36.000 | 1/1 / 0/1 |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/result.json) / repetitive | passed | 1 | 191.741 | 26.474 | 26.474 | 2.609 | 19.078 | 0/1 / 0/1 |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/result.json) / copy_source | passed | 1 | 272.883 | 25.651 | 25.651 | 3.618 | 40.147 | 1/1 / 0/1 |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/result.json) / copy_repetitive | passed | 1 | 189.116 | 28.653 | 28.653 | 2.608 | 18.069 | 1/1 / 0/1 |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/result.json) / prose | passed | 1 | 63.204 | 29.077 | 29.077 | 1.172 | 9.942 | — / — |
| [ngram-ccd-t12-0](runs/ngram-ccd-t12-0/result.json) / edit | passed | 1 | 275.449 | 49.757 | 49.757 | 3.653 | 22.485 | 1/1 / 0/1 |
| [ngram-ccd-t12-0](runs/ngram-ccd-t12-0/result.json) / prose | passed | 1 | 59.919 | 26.747 | 26.747 | 1.236 | 10.770 | — / — |
| [ngram-ccd-t12-0](runs/ngram-ccd-t12-0/result.json) / repetitive | passed | 1 | 184.642 | 23.343 | 23.343 | 2.709 | 21.387 | 0/1 / 0/1 |
| [ngram-ccd-t12-1](runs/ngram-ccd-t12-1/result.json) / edit | passed | 1 | 276.533 | 49.738 | 49.738 | 3.639 | 22.478 | 1/1 / 0/1 |
| [ngram-ccd-t12-1](runs/ngram-ccd-t12-1/result.json) / prose | passed | 1 | 60.916 | 26.841 | 26.841 | 1.216 | 10.716 | — / — |
| [ngram-ccd-t12-1](runs/ngram-ccd-t12-1/result.json) / repetitive | passed | 1 | 189.595 | 21.760 | 21.760 | 2.638 | 22.675 | 0/1 / 0/1 |
| [ngram-ccd-t6-0](runs/ngram-ccd-t6-0/result.json) / edit | passed | 1 | 272.329 | 37.247 | 37.247 | 3.695 | 28.852 | 1/1 / 0/1 |
| [ngram-ccd-t6-0](runs/ngram-ccd-t6-0/result.json) / prose | passed | 1 | 58.469 | 26.575 | 26.575 | 1.267 | 10.862 | — / — |
| [ngram-ccd-t6-0](runs/ngram-ccd-t6-0/result.json) / repetitive | passed | 1 | 183.997 | 17.643 | 17.643 | 2.718 | 27.430 | 0/1 / 0/1 |
| [ngram-ccd-t6-1](runs/ngram-ccd-t6-1/result.json) / edit | passed | 1 | 273.522 | 37.083 | 37.083 | 3.679 | 28.947 | 1/1 / 0/1 |
| [ngram-ccd-t6-1](runs/ngram-ccd-t6-1/result.json) / prose | passed | 1 | 62.221 | 28.934 | 28.934 | 1.190 | 10.003 | — / — |
| [ngram-ccd-t6-1](runs/ngram-ccd-t6-1/result.json) / repetitive | passed | 1 | 194.797 | 17.798 | 17.798 | 2.568 | 27.066 | 0/1 / 0/1 |
| [ngram-rs-4-enabled0-0](runs/ngram-rs-4-enabled0-0/result.json) / edit | passed | 1 | 266.618 | 30.079 | 30.079 | 3.774 | 34.927 | 1/1 / 0/1 |
| [ngram-rs-4-enabled0-0](runs/ngram-rs-4-enabled0-0/result.json) / prose | passed | 1 | 55.828 | 27.119 | 27.119 | 1.327 | 10.729 | — / — |
| [ngram-rs-4-enabled0-0](runs/ngram-rs-4-enabled0-0/result.json) / repetitive | passed | 1 | 184.828 | 24.091 | 24.091 | 2.706 | 20.805 | 0/1 / 0/1 |
| [ngram-rs-4-enabled0-1](runs/ngram-rs-4-enabled0-1/result.json) / edit | passed | 1 | 263.614 | 31.379 | 31.379 | 3.817 | 33.679 | 1/1 / 0/1 |
| [ngram-rs-4-enabled0-1](runs/ngram-rs-4-enabled0-1/result.json) / prose | passed | 1 | 55.475 | 26.166 | 26.166 | 1.336 | 11.080 | — / — |
| [ngram-rs-4-enabled0-1](runs/ngram-rs-4-enabled0-1/result.json) / repetitive | passed | 1 | 174.496 | 20.048 | 20.048 | 2.867 | 24.615 | 0/1 / 0/1 |
| [ngram-rs-4-enabled1-0](runs/ngram-rs-4-enabled1-0/result.json) / edit | passed | 1 | 280.074 | 42.792 | 42.792 | 3.593 | 25.490 | 1/1 / 0/1 |
| [ngram-rs-4-enabled1-0](runs/ngram-rs-4-enabled1-0/result.json) / prose | passed | 1 | 60.221 | 28.061 | 28.061 | 1.231 | 10.317 | — / — |
| [ngram-rs-4-enabled1-0](runs/ngram-rs-4-enabled1-0/result.json) / repetitive | passed | 1 | 186.480 | 27.931 | 27.931 | 2.682 | 18.292 | 0/1 / 0/1 |
| [ngram-rs-4-enabled1-1](runs/ngram-rs-4-enabled1-1/result.json) / edit | passed | 1 | 261.544 | 35.842 | 35.842 | 3.848 | 29.991 | 1/1 / 0/1 |
| [ngram-rs-4-enabled1-1](runs/ngram-rs-4-enabled1-1/result.json) / prose | passed | 1 | 55.432 | 25.505 | 25.505 | 1.337 | 11.335 | — / — |
| [ngram-rs-4-enabled1-1](runs/ngram-rs-4-enabled1-1/result.json) / repetitive | passed | 1 | 176.318 | 23.689 | 23.689 | 2.837 | 21.242 | 0/1 / 0/1 |
| [ngram-rs-8-enabled0-0](runs/ngram-rs-8-enabled0-0/result.json) / edit | passed | 1 | 280.530 | 45.377 | 45.377 | 3.587 | 24.237 | 1/1 / 0/1 |
| [ngram-rs-8-enabled0-0](runs/ngram-rs-8-enabled0-0/result.json) / prose | passed | 1 | 59.397 | 24.479 | 24.479 | 1.248 | 11.665 | — / — |
| [ngram-rs-8-enabled0-0](runs/ngram-rs-8-enabled0-0/result.json) / repetitive | passed | 1 | 186.498 | 22.219 | 22.219 | 2.682 | 22.305 | 0/1 / 0/1 |
| [ngram-rs-8-enabled0-1](runs/ngram-rs-8-enabled0-1/result.json) / edit | passed | 1 | 264.537 | 37.108 | 37.108 | 3.804 | 29.055 | 1/1 / 0/1 |
| [ngram-rs-8-enabled0-1](runs/ngram-rs-8-enabled0-1/result.json) / prose | passed | 1 | 56.305 | 26.018 | 26.018 | 1.316 | 11.116 | — / — |
| [ngram-rs-8-enabled0-1](runs/ngram-rs-8-enabled0-1/result.json) / repetitive | passed | 1 | 176.419 | 20.115 | 20.115 | 2.836 | 24.511 | 0/1 / 0/1 |
| [ngram-rs-8-enabled1-0](runs/ngram-rs-8-enabled1-0/result.json) / edit | passed | 1 | 276.838 | 46.036 | 46.036 | 3.635 | 23.989 | 1/1 / 0/1 |
| [ngram-rs-8-enabled1-0](runs/ngram-rs-8-enabled1-0/result.json) / prose | passed | 1 | 53.246 | 24.256 | 24.256 | 1.391 | 11.904 | — / — |
| [ngram-rs-8-enabled1-0](runs/ngram-rs-8-enabled1-0/result.json) / repetitive | passed | 1 | 175.323 | 22.991 | 22.991 | 2.853 | 21.817 | 0/1 / 0/1 |
| [ngram-rs-8-enabled1-1](runs/ngram-rs-8-enabled1-1/result.json) / edit | passed | 1 | 264.612 | 37.940 | 37.940 | 3.803 | 28.501 | 1/1 / 0/1 |
| [ngram-rs-8-enabled1-1](runs/ngram-rs-8-enabled1-1/result.json) / prose | passed | 1 | 56.841 | 24.576 | 24.576 | 1.303 | 11.679 | — / — |
| [ngram-rs-8-enabled1-1](runs/ngram-rs-8-enabled1-1/result.json) / repetitive | passed | 1 | 173.293 | 23.834 | 23.834 | 2.887 | 21.180 | 0/1 / 0/1 |
| [original-after-turbo](runs/original-after-turbo/result.json) / review | passed | 3 | 299.854 | 28.004 | 28.004 | 20.846 | 29.952 | — / — |
| [original-after-turbo](runs/original-after-turbo/result.json) / prose | passed | 3 | 60.587 | 29.018 | 29.018 | 1.222 | 10.010 | — / — |
| [original-after-turbo](runs/original-after-turbo/result.json) / edit | passed | 3 | 295.905 | 28.920 | 28.920 | 3.401 | 35.800 | 3/3 / 0/3 |
| [original-after-turbo](runs/original-after-turbo/result.json) / conversation | passed | 3 | 66.910 | 28.246 | 28.246 | 0.974 | 9.995 | — / — |
| [original-api](runs/original-api/result.json) / prose | passed | 1 | 56.570 | 29.195 | 29.195 | 1.309 | 10.044 | — / — |
| [original-controlled-load-0](runs/original-controlled-load-0/result.json) / prose | passed | 1 | 55.046 | 28.495 | 28.495 | 1.346 | 3.556 | — / — |
| [original-controlled-load-1](runs/original-controlled-load-1/result.json) / prose | passed | 1 | 55.127 | 28.568 | 28.568 | 1.344 | 3.548 | — / — |
| [original-threads24](runs/original-threads24/result.json) / review | passed | 2 | 294.778 | 23.148 | 23.148 | 21.217 | 32.235 | — / — |
| [original-threads24](runs/original-threads24/result.json) / prose | passed | 2 | 63.300 | 24.289 | 24.289 | 1.170 | 11.672 | — / — |
| [original-threads24](runs/original-threads24/result.json) / edit | passed | 2 | 300.539 | 24.620 | 24.620 | 3.348 | 41.407 | 2/2 / 0/2 |
| [parallel-original](runs/parallel-original/result.json) / review | passed | 2 | 299.725 | 28.104 | 28.104 | 20.855 | 29.929 | — / — |
| [parallel-original](runs/parallel-original/result.json) / prose | passed | 2 | 59.121 | 29.227 | 29.227 | 1.253 | 9.978 | — / — |
| [parallel-original](runs/parallel-original/result.json) / edit | passed | 2 | 295.233 | 29.162 | 29.162 | 3.409 | 35.540 | 2/2 / 0/2 |
| [place-down12](runs/place-down12/result.json) / review | passed | 2 | 297.487 | 27.543 | 27.543 | 21.013 | 30.271 | — / — |
| [place-down12](runs/place-down12/result.json) / prose | passed | 2 | 59.421 | 28.424 | 28.424 | 1.246 | 10.218 | — / — |
| [place-down12](runs/place-down12/result.json) / edit | passed | 2 | 295.149 | 28.440 | 28.440 | 3.410 | 36.356 | 2/2 / 0/2 |
| [place-gateup14](runs/place-gateup14/result.json) / review | passed | 2 | 299.577 | 28.031 | 28.031 | 20.866 | 29.963 | — / — |
| [place-gateup14](runs/place-gateup14/result.json) / prose | passed | 2 | 59.479 | 29.034 | 29.034 | 1.246 | 10.028 | — / — |
| [place-gateup14](runs/place-gateup14/result.json) / edit | passed | 2 | 297.337 | 28.963 | 28.963 | 3.385 | 35.736 | 2/2 / 0/2 |
| [place-gateup16](runs/place-gateup16/result.json) / review | passed | 2 | 305.318 | 28.294 | 28.294 | 20.474 | 29.486 | — / — |
| [place-gateup16](runs/place-gateup16/result.json) / prose | passed | 2 | 60.405 | 29.283 | 29.283 | 1.227 | 9.935 | — / — |
| [place-gateup16](runs/place-gateup16/result.json) / edit | passed | 2 | 301.056 | 29.101 | 29.101 | 3.344 | 35.541 | 2/2 / 0/2 |
| [place-mid7](runs/place-mid7/result.json) / review | passed | 2 | 286.408 | 25.828 | 25.828 | 21.826 | 31.698 | — / — |
| [place-mid7](runs/place-mid7/result.json) / prose | passed | 2 | 61.151 | 29.087 | 29.087 | 1.211 | 9.978 | — / — |
| [place-mid7](runs/place-mid7/result.json) / edit | passed | 2 | 302.858 | 29.229 | 29.229 | 3.323 | 35.381 | 2/2 / 0/2 |
| [place-mid8](runs/place-mid8/result.json) / review | passed | 2 | 311.010 | 28.841 | 28.841 | 20.099 | 28.940 | — / — |
| [place-mid8](runs/place-mid8/result.json) / prose | passed | 2 | 62.547 | 29.921 | 29.921 | 1.185 | 9.706 | — / — |
| [place-mid8](runs/place-mid8/result.json) / edit | passed | 2 | 307.748 | 29.548 | 29.548 | 3.271 | 34.983 | 2/2 / 0/2 |
| [poll0](runs/poll0/result.json) / review | passed | 2 | 300.886 | 27.674 | 27.674 | 20.775 | 29.990 | — / — |
| [poll0](runs/poll0/result.json) / prose | passed | 2 | 60.319 | 28.750 | 28.750 | 1.228 | 10.097 | — / — |
| [poll0](runs/poll0/result.json) / edit | passed | 2 | 294.779 | 28.706 | 28.706 | 3.414 | 36.055 | 2/2 / 0/2 |
| [spec-original-0-edit-0](runs/spec-original-0-edit-0/result.json) / edit | passed | 1 | 286.896 | 29.071 | 29.071 | 3.508 | 35.739 | 1/1 / 0/1 |
| [spec-original-0-edit-1](runs/spec-original-0-edit-1/result.json) / edit | passed | 1 | 284.640 | 28.551 | 28.551 | 3.535 | 36.354 | 1/1 / 0/1 |
| [spec-original-0-prose-0](runs/spec-original-0-prose-0/result.json) / prose | passed | 1 | 55.670 | 29.030 | 29.030 | 1.330 | 10.114 | — / — |
| [spec-original-0-prose-1](runs/spec-original-0-prose-1/result.json) / prose | passed | 1 | 55.907 | 28.703 | 28.703 | 1.324 | 10.209 | — / — |
| [spec-original-0-reasoning-0](runs/spec-original-0-reasoning-0/result.json) / reasoning | passed | 1 | 71.232 | 28.945 | 28.945 | 1.545 | 10.355 | — / — |
| [spec-original-0-reasoning-1](runs/spec-original-0-reasoning-1/result.json) / reasoning | passed | 1 | 71.468 | 28.745 | 28.745 | 1.540 | 10.411 | — / — |
| [spec-original-0-repetitive-0](runs/spec-original-0-repetitive-0/result.json) / repetitive | passed | 1 | 184.053 | 29.076 | 29.076 | 2.717 | 17.713 | 0/1 / 0/1 |
| [spec-original-0-repetitive-1](runs/spec-original-0-repetitive-1/result.json) / repetitive | passed | 1 | 181.738 | 28.601 | 28.601 | 2.752 | 17.996 | 0/1 / 0/1 |
| [spec-original-16-edit-0](runs/spec-original-16-edit-0/result.json) / edit | passed | 1 | 277.182 | 41.206 | 41.206 | 3.630 | 26.370 | 1/1 / 0/1 |
| [spec-original-16-edit-1](runs/spec-original-16-edit-1/result.json) / edit | passed | 1 | 283.710 | 49.557 | 49.557 | 3.547 | 22.455 | 1/1 / 0/1 |
| [spec-original-16-prose-0](runs/spec-original-16-prose-0/result.json) / prose | passed | 1 | 55.919 | 28.228 | 28.228 | 1.324 | 10.358 | — / — |
| [spec-original-16-prose-1](runs/spec-original-16-prose-1/result.json) / prose | passed | 1 | 55.588 | 28.698 | 28.698 | 1.332 | 10.218 | — / — |
| [spec-original-16-reasoning-0](runs/spec-original-16-reasoning-0/result.json) / reasoning | passed | 1 | 65.659 | 28.086 | 28.086 | 1.676 | 10.756 | — / — |
| [spec-original-16-reasoning-1](runs/spec-original-16-reasoning-1/result.json) / reasoning | passed | 1 | 71.315 | 28.485 | 28.485 | 1.543 | 10.496 | — / — |
| [spec-original-16-repetitive-0](runs/spec-original-16-repetitive-0/result.json) / repetitive | passed | 1 | 179.858 | 19.350 | 19.350 | 2.781 | 25.313 | 0/1 / 0/1 |
| [spec-original-16-repetitive-1](runs/spec-original-16-repetitive-1/result.json) / repetitive | passed | 1 | 183.286 | 20.888 | 20.888 | 2.729 | 23.602 | 0/1 / 0/1 |
| [spec-original-4-edit-0](runs/spec-original-4-edit-0/result.json) / edit | passed | 1 | 285.451 | 39.248 | 39.248 | 3.525 | 27.399 | 1/1 / 0/1 |
| [spec-original-4-edit-1](runs/spec-original-4-edit-1/result.json) / edit | passed | 1 | 282.951 | 38.146 | 38.146 | 3.556 | 28.120 | 1/1 / 0/1 |
| [spec-original-4-prose-0](runs/spec-original-4-prose-0/result.json) / prose | passed | 1 | 56.463 | 29.182 | 29.182 | 1.311 | 10.050 | — / — |
| [spec-original-4-prose-1](runs/spec-original-4-prose-1/result.json) / prose | passed | 1 | 55.734 | 28.551 | 28.551 | 1.328 | 10.260 | — / — |
| [spec-original-4-reasoning-0](runs/spec-original-4-reasoning-0/result.json) / reasoning | passed | 1 | 72.402 | 29.030 | 29.030 | 1.520 | 10.304 | — / — |
| [spec-original-4-reasoning-1](runs/spec-original-4-reasoning-1/result.json) / reasoning | passed | 1 | 71.171 | 28.290 | 28.290 | 1.546 | 10.560 | — / — |
| [spec-original-4-repetitive-0](runs/spec-original-4-repetitive-0/result.json) / repetitive | passed | 1 | 185.342 | 23.957 | 23.957 | 2.699 | 20.898 | 0/1 / 0/1 |
| [spec-original-4-repetitive-1](runs/spec-original-4-repetitive-1/result.json) / repetitive | passed | 1 | 183.968 | 23.889 | 23.889 | 2.719 | 20.970 | 0/1 / 0/1 |
| [spec-original-8-edit-0](runs/spec-original-8-edit-0/result.json) / edit | passed | 1 | 278.422 | 37.937 | 37.937 | 3.614 | 28.313 | 1/1 / 0/1 |
| [spec-original-8-edit-1](runs/spec-original-8-edit-1/result.json) / edit | passed | 1 | 284.097 | 42.996 | 42.996 | 3.542 | 25.335 | 1/1 / 0/1 |
| [spec-original-8-prose-0](runs/spec-original-8-prose-0/result.json) / prose | passed | 1 | 55.813 | 28.554 | 28.554 | 1.327 | 10.257 | — / — |
| [spec-original-8-prose-1](runs/spec-original-8-prose-1/result.json) / prose | passed | 1 | 55.101 | 28.633 | 28.633 | 1.344 | 10.250 | — / — |
| [spec-original-8-reasoning-0](runs/spec-original-8-reasoning-0/result.json) / reasoning | passed | 1 | 71.005 | 28.214 | 28.214 | 1.550 | 10.588 | — / — |
| [spec-original-8-reasoning-1](runs/spec-original-8-reasoning-1/result.json) / reasoning | passed | 1 | 72.392 | 28.562 | 28.562 | 1.520 | 10.448 | — / — |
| [spec-original-8-repetitive-0](runs/spec-original-8-repetitive-0/result.json) / repetitive | passed | 1 | 179.405 | 21.894 | 21.894 | 2.788 | 22.702 | 0/1 / 0/1 |
| [spec-original-8-repetitive-1](runs/spec-original-8-repetitive-1/result.json) / repetitive | passed | 1 | 184.049 | 23.069 | 23.069 | 2.717 | 21.618 | 0/1 / 0/1 |
| [spec-thread-16-t12-0](runs/spec-thread-16-t12-0/result.json) / edit | passed | 1 | 280.978 | 53.159 | 53.159 | 3.581 | 21.208 | 1/1 / 0/1 |
| [spec-thread-16-t12-1](runs/spec-thread-16-t12-1/result.json) / edit | passed | 1 | 286.465 | 54.162 | 54.162 | 3.513 | 20.813 | 1/1 / 0/1 |
| [spec-thread-16-t24-0](runs/spec-thread-16-t24-0/result.json) / edit | passed | 1 | 283.257 | 51.749 | 51.749 | 3.552 | 21.660 | 1/1 / 0/1 |
| [spec-thread-16-t24-1](runs/spec-thread-16-t24-1/result.json) / edit | passed | 1 | 283.954 | 51.520 | 51.520 | 3.544 | 21.731 | 1/1 / 0/1 |
| [spec-thread-4-t12-0](runs/spec-thread-4-t12-0/result.json) / edit | passed | 1 | 283.640 | 42.470 | 42.470 | 3.548 | 25.611 | 1/1 / 0/1 |
| [spec-thread-4-t12-1](runs/spec-thread-4-t12-1/result.json) / edit | passed | 1 | 286.180 | 39.631 | 39.631 | 3.516 | 27.159 | 1/1 / 0/1 |
| [spec-thread-4-t24-0](runs/spec-thread-4-t24-0/result.json) / edit | passed | 1 | 278.263 | 37.910 | 37.910 | 3.616 | 28.333 | 1/1 / 0/1 |
| [spec-thread-4-t24-1](runs/spec-thread-4-t24-1/result.json) / edit | passed | 1 | 282.154 | 38.954 | 38.954 | 3.566 | 27.621 | 1/1 / 0/1 |
| [spec-thread-8-t12-0](runs/spec-thread-8-t12-0/result.json) / edit | passed | 1 | 283.896 | 48.656 | 48.656 | 3.544 | 22.803 | 1/1 / 0/1 |
| [spec-thread-8-t12-1](runs/spec-thread-8-t12-1/result.json) / edit | passed | 1 | 286.216 | 48.716 | 48.716 | 3.516 | 22.750 | 1/1 / 0/1 |
| [spec-thread-8-t24-0](runs/spec-thread-8-t24-0/result.json) / edit | passed | 1 | 282.261 | 41.704 | 41.704 | 3.565 | 26.033 | 1/1 / 0/1 |
| [spec-thread-8-t24-1](runs/spec-thread-8-t24-1/result.json) / edit | passed | 1 | 286.307 | 45.647 | 45.647 | 3.515 | 24.042 | 1/1 / 0/1 |
| [stage-slot64-repeat](runs/stage-slot64-repeat/result.json) / review | passed | 3 | 303.373 | 26.688 | 26.688 | 20.604 | 30.259 | — / — |
| [stage-slot64-repeat](runs/stage-slot64-repeat/result.json) / prose | passed | 3 | 75.116 | 29.256 | 29.256 | 0.986 | 9.714 | — / — |
| [stage-slot64-repeat](runs/stage-slot64-repeat/result.json) / edit | passed | 3 | 282.536 | 25.371 | 25.371 | 3.562 | 40.494 | 3/3 / 0/3 |
| [stage-slot8-repeat](runs/stage-slot8-repeat/result.json) / review | passed | 3 | 331.826 | 28.115 | 28.115 | 18.838 | 28.689 | — / — |
| [stage-slot8-repeat](runs/stage-slot8-repeat/result.json) / prose | passed | 3 | 68.690 | 27.515 | 27.515 | 1.078 | 10.346 | — / — |
| [stage-slot8-repeat](runs/stage-slot8-repeat/result.json) / edit | passed | 3 | 298.817 | 27.717 | 27.717 | 3.368 | 36.844 | 3/3 / 0/3 |
| [state-equivalence-fork-cacheoff](runs/state-equivalence-fork-cacheoff/result.json) | passed | 0 | — | — | — | — | — | — |
| [state-equivalence-original](runs/state-equivalence-original/result.json) | passed | 0 | — | — | — | — | — | — |
| [threads12-batch12](runs/threads12-batch12/result.json) / review | passed | 2 | 299.715 | 28.036 | 28.036 | 20.856 | 29.952 | — / — |
| [threads12-batch12](runs/threads12-batch12/result.json) / prose | passed | 2 | 63.404 | 29.174 | 29.174 | 1.168 | 9.909 | — / — |
| [threads12-batch12](runs/threads12-batch12/result.json) / edit | passed | 2 | 300.914 | 29.016 | 29.016 | 3.344 | 35.637 | 2/2 / 0/2 |
| [threads6-batch12](runs/threads6-batch12/result.json) / review | passed | 2 | 299.981 | 24.206 | 24.206 | 20.838 | 31.373 | — / — |
| [threads6-batch12](runs/threads6-batch12/result.json) / prose | passed | 2 | 62.222 | 25.147 | 25.147 | 1.190 | 11.330 | — / — |
| [threads6-batch12](runs/threads6-batch12/result.json) / edit | passed | 2 | 298.838 | 25.287 | 25.287 | 3.367 | 40.422 | 2/2 / 0/2 |
| [threads8-batch12](runs/threads8-batch12/result.json) / review | passed | 2 | 301.097 | 26.175 | 26.175 | 20.760 | 30.503 | — / — |
| [threads8-batch12](runs/threads8-batch12/result.json) / prose | passed | 2 | 63.098 | 26.795 | 26.795 | 1.174 | 10.691 | — / — |
| [threads8-batch12](runs/threads8-batch12/result.json) / edit | passed | 2 | 299.751 | 26.744 | 26.744 | 3.357 | 38.393 | 2/2 / 0/2 |
| [turbo-cache-4480-cpu48](runs/turbo-cache-4480-cpu48/result.json) / review | passed | 3 | 273.088 | 22.350 | 22.263 | 22.889 | 34.343 | — / — |
| [turbo-cache-4480-cpu48](runs/turbo-cache-4480-cpu48/result.json) / prose | passed | 3 | 49.479 | 21.608 | 21.524 | 1.499 | 13.344 | — / — |
| [turbo-cache-4480-cpu48](runs/turbo-cache-4480-cpu48/result.json) / edit | passed | 3 | 261.340 | 19.801 | 19.780 | 3.852 | 51.044 | 3/3 / 0/3 |
| [turbo-cache-6000-cpu48](runs/turbo-cache-6000-cpu48/result.json) / review | passed | 3 | 278.590 | 24.034 | 23.940 | 22.437 | 33.089 | — / — |
| [turbo-cache-6000-cpu48](runs/turbo-cache-6000-cpu48/result.json) / prose | passed | 3 | 45.463 | 22.906 | 22.816 | 1.631 | 12.739 | — / — |
| [turbo-cache-6000-cpu48](runs/turbo-cache-6000-cpu48/result.json) / edit | passed | 3 | 248.675 | 21.799 | 21.775 | 4.047 | 47.239 | 3/3 / 0/3 |
| [turbo-cache-6000-cpu48-min256](runs/turbo-cache-6000-cpu48-min256/result.json) / review | passed | 3 | 274.391 | 22.919 | 22.830 | 22.782 | 33.950 | — / — |
| [turbo-cache-6000-cpu48-min256](runs/turbo-cache-6000-cpu48-min256/result.json) / prose | passed | 3 | 53.918 | 23.212 | 23.121 | 1.374 | 12.401 | — / — |
| [turbo-cache-6000-cpu48-min256](runs/turbo-cache-6000-cpu48-min256/result.json) / edit | passed | 3 | 253.843 | 19.809 | 19.788 | 3.966 | 51.035 | 3/3 / 0/3 |
| [turbo-cache-off-cpu42](runs/turbo-cache-off-cpu42/result.json) / review | excluded | 3 | 302.955 | 19.228 | 19.153 | 20.633 | 33.556 | — / — |
| [turbo-cache-off-cpu42](runs/turbo-cache-off-cpu42/result.json) / prose | excluded | 3 | 45.122 | 21.628 | 21.543 | 1.641 | 13.401 | — / — |
| [turbo-cache-off-cpu42](runs/turbo-cache-off-cpu42/result.json) / edit | excluded | 3 | 280.031 | 22.480 | 22.456 | 3.594 | 45.997 | 3/3 / 0/3 |
| [turbo-cache-off-cpu48](runs/turbo-cache-off-cpu48/result.json) / review | passed | 3 | 270.538 | 17.784 | 17.715 | 23.107 | 37.350 | — / — |
| [turbo-cache-off-cpu48](runs/turbo-cache-off-cpu48/result.json) / prose | passed | 3 | 40.461 | 20.374 | 20.295 | 1.830 | 14.395 | — / — |
| [turbo-cache-off-cpu48](runs/turbo-cache-off-cpu48/result.json) / edit | passed | 3 | 271.169 | 20.436 | 20.414 | 3.712 | 49.611 | 3/3 / 0/3 |
| [turbo-combined-control](runs/turbo-combined-control/result.json) / review | passed | 2 | 367.739 | 32.080 | 31.954 | 17.003 | 24.983 | — / — |
| [turbo-combined-control](runs/turbo-combined-control/result.json) / prose | passed | 2 | 54.837 | 30.159 | 30.041 | 1.353 | 9.856 | — / — |
| [turbo-combined-control](runs/turbo-combined-control/result.json) / edit | passed | 2 | 290.069 | 23.489 | 23.464 | 3.471 | 43.426 | 2/2 / 0/2 |
| [turbo-combined-cpu42-cache1024](runs/turbo-combined-cpu42-cache1024/result.json) / review | passed | 2 | 402.257 | 28.299 | 28.189 | 15.542 | 24.589 | — / — |
| [turbo-combined-cpu42-cache1024](runs/turbo-combined-cpu42-cache1024/result.json) / prose | passed | 2 | 58.424 | 28.033 | 27.923 | 1.275 | 10.412 | — / — |
| [turbo-combined-cpu42-cache1024](runs/turbo-combined-cpu42-cache1024/result.json) / edit | passed | 2 | 310.462 | 25.648 | 25.621 | 3.241 | 39.813 | 2/2 / 0/2 |
| [turbo-combined-cpu44-cache3072](runs/turbo-combined-cpu44-cache3072/result.json) / review | passed | 2 | 376.412 | 28.616 | 28.504 | 16.607 | 25.555 | — / — |
| [turbo-combined-cpu44-cache3072](runs/turbo-combined-cpu44-cache3072/result.json) / prose | passed | 2 | 57.003 | 29.713 | 29.597 | 1.300 | 9.915 | — / — |
| [turbo-combined-cpu44-cache3072](runs/turbo-combined-cpu44-cache3072/result.json) / edit | passed | 2 | 298.800 | 25.116 | 25.090 | 3.368 | 40.716 | 2/2 / 0/2 |
| [turbo-combined-cpu46-cache4480](runs/turbo-combined-cpu46-cache4480/result.json) / review | passed | 2 | 362.592 | 31.205 | 31.083 | 17.241 | 25.449 | — / — |
| [turbo-combined-cpu46-cache4480](runs/turbo-combined-cpu46-cache4480/result.json) / prose | passed | 2 | 54.716 | 30.736 | 30.616 | 1.354 | 9.687 | — / — |
| [turbo-combined-cpu46-cache4480](runs/turbo-combined-cpu46-cache4480/result.json) / edit | passed | 2 | 287.766 | 24.471 | 24.445 | 3.498 | 41.828 | 2/2 / 0/2 |
| [turbo-combined-min256](runs/turbo-combined-min256/result.json) / review | passed | 2 | 356.538 | 33.486 | 33.355 | 17.541 | 25.192 | — / — |
| [turbo-combined-min256](runs/turbo-combined-min256/result.json) / prose | passed | 2 | 53.662 | 32.068 | 31.943 | 1.381 | 9.363 | — / — |
| [turbo-combined-min256](runs/turbo-combined-min256/result.json) / edit | passed | 2 | 290.968 | 21.994 | 21.971 | 3.461 | 46.109 | 2/2 / 0/2 |
| [turbo-combined-ngram16-t12](runs/turbo-combined-ngram16-t12/result.json) / prose | passed | 2 | 54.975 | 36.040 | 35.900 | 1.351 | 8.477 | — / — |
| [turbo-combined-ngram16-t12](runs/turbo-combined-ngram16-t12/result.json) / edit | passed | 2 | 292.295 | 47.720 | 47.669 | 3.444 | 23.105 | 2/2 / 0/2 |
| [turbo-combined-ngram16-t12](runs/turbo-combined-ngram16-t12/result.json) / repetitive | passed | 2 | 180.416 | 21.949 | 21.898 | 2.775 | 23.129 | 0/2 / 0/2 |
| [turbo-combined-ngram4-t12](runs/turbo-combined-ngram4-t12/result.json) / prose | passed | 2 | 51.839 | 37.174 | 37.029 | 1.434 | 8.332 | — / — |
| [turbo-combined-ngram4-t12](runs/turbo-combined-ngram4-t12/result.json) / edit | passed | 2 | 282.776 | 31.875 | 31.841 | 3.559 | 32.986 | 2/2 / 0/2 |
| [turbo-combined-ngram4-t12](runs/turbo-combined-ngram4-t12/result.json) / repetitive | passed | 2 | 179.464 | 21.991 | 21.941 | 2.789 | 22.827 | 0/2 / 0/2 |
| [turbo-combined-ngram4-t24](runs/turbo-combined-ngram4-t24/result.json) / prose | passed | 2 | 50.882 | 36.080 | 35.939 | 1.465 | 8.560 | — / — |
| [turbo-combined-ngram4-t24](runs/turbo-combined-ngram4-t24/result.json) / edit | passed | 2 | 275.264 | 22.490 | 22.466 | 3.659 | 45.375 | 2/2 / 0/2 |
| [turbo-combined-ngram4-t24](runs/turbo-combined-ngram4-t24/result.json) / repetitive | passed | 2 | 172.238 | 14.203 | 14.171 | 2.906 | 33.897 | 0/2 / 0/2 |
| [turbo-cow-enabled0](runs/turbo-cow-enabled0/result.json) / review | passed | 3 | 373.621 | 31.356 | 31.233 | 16.732 | 24.832 | — / — |
| [turbo-cow-enabled0](runs/turbo-cow-enabled0/result.json) / prose | passed | 3 | 57.586 | 31.443 | 31.321 | 1.287 | 9.459 | — / — |
| [turbo-cow-enabled0](runs/turbo-cow-enabled0/result.json) / edit | passed | 3 | 289.407 | 24.348 | 24.322 | 3.479 | 42.039 | 3/3 / 0/3 |
| [turbo-cow-enabled1](runs/turbo-cow-enabled1/result.json) / review | passed | 3 | 525.149 | 29.972 | 29.855 | 11.904 | 20.445 | — / — |
| [turbo-cow-enabled1](runs/turbo-cow-enabled1/result.json) / prose | passed | 3 | 88.253 | 31.058 | 30.937 | 0.841 | 9.117 | — / — |
| [turbo-cow-enabled1](runs/turbo-cow-enabled1/result.json) / edit | passed | 3 | 426.675 | 24.417 | 24.391 | 2.360 | 40.784 | 3/3 / 0/3 |
| [turbo-cow-ngram16-t12](runs/turbo-cow-ngram16-t12/result.json) / prose | passed | 2 | 73.747 | 30.312 | 30.194 | 1.056 | 9.518 | — / — |
| [turbo-cow-ngram16-t12](runs/turbo-cow-ngram16-t12/result.json) / edit | passed | 2 | 339.215 | 40.104 | 40.061 | 3.114 | 26.553 | 2/2 / 0/2 |
| [turbo-cow-ngram16-t12](runs/turbo-cow-ngram16-t12/result.json) / repetitive | passed | 2 | 258.554 | 20.086 | 20.040 | 1.939 | 24.464 | 0/2 / 0/2 |
| [turbo-flat-cache](runs/turbo-flat-cache/result.json) / review | passed | 3 | 295.057 | 30.899 | 30.778 | 21.185 | 29.562 | — / — |
| [turbo-flat-cache](runs/turbo-flat-cache/result.json) / prose | passed | 3 | 56.008 | 30.249 | 30.130 | 1.323 | 9.785 | — / — |
| [turbo-flat-cache](runs/turbo-flat-cache/result.json) / edit | passed | 3 | 294.566 | 23.593 | 23.568 | 3.417 | 43.174 | 3/3 / 0/3 |
| [turbo-flat-cache](runs/turbo-flat-cache/result.json) / conversation | passed | 3 | 59.584 | 27.738 | 27.630 | 1.094 | 10.323 | — / — |
| [turbo-followup-async-fill](runs/turbo-followup-async-fill/result.json) / review | passed | 2 | 293.107 | 31.586 | 31.463 | 21.328 | 29.433 | — / — |
| [turbo-followup-async-fill](runs/turbo-followup-async-fill/result.json) / prose | passed | 2 | 55.742 | 31.684 | 31.560 | 1.329 | 9.411 | — / — |
| [turbo-followup-async-fill](runs/turbo-followup-async-fill/result.json) / edit | passed | 2 | 293.722 | 24.994 | 24.967 | 3.427 | 40.956 | 2/2 / 0/2 |
| [turbo-followup-batch1536](runs/turbo-followup-batch1536/result.json) / review | passed | 2 | 371.017 | 31.632 | 31.508 | 16.851 | 24.944 | — / — |
| [turbo-followup-batch1536](runs/turbo-followup-batch1536/result.json) / prose | passed | 2 | 56.478 | 33.426 | 33.296 | 1.312 | 8.970 | — / — |
| [turbo-followup-batch1536](runs/turbo-followup-batch1536/result.json) / edit | passed | 2 | 294.917 | 24.830 | 24.803 | 3.413 | 41.191 | 2/2 / 0/2 |
| [turbo-followup-batch2048-cache4480](runs/turbo-followup-batch2048-cache4480/result.json) / review | passed | 2 | 441.877 | 31.831 | 31.707 | 14.150 | 22.195 | — / — |
| [turbo-followup-batch2048-cache4480](runs/turbo-followup-batch2048-cache4480/result.json) / prose | passed | 2 | 54.372 | 31.899 | 31.774 | 1.364 | 9.390 | — / — |
| [turbo-followup-batch2048-cache4480](runs/turbo-followup-batch2048-cache4480/result.json) / edit | passed | 2 | 295.535 | 24.771 | 24.745 | 3.406 | 41.272 | 2/2 / 0/2 |
| [turbo-followup-low-churn](runs/turbo-followup-low-churn/result.json) / review | passed | 2 | 294.879 | 30.560 | 30.441 | 21.199 | 29.589 | — / — |
| [turbo-followup-low-churn](runs/turbo-followup-low-churn/result.json) / prose | passed | 2 | 56.897 | 31.508 | 31.385 | 1.302 | 9.426 | — / — |
| [turbo-followup-low-churn](runs/turbo-followup-low-churn/result.json) / edit | passed | 2 | 294.905 | 26.154 | 26.127 | 3.414 | 39.277 | 2/2 / 0/2 |
| [turbo-followup-ngram2](runs/turbo-followup-ngram2/result.json) / prose | passed | 2 | 53.095 | 32.654 | 32.526 | 1.400 | 9.239 | — / — |
| [turbo-followup-ngram2](runs/turbo-followup-ngram2/result.json) / edit | passed | 2 | 295.067 | 24.311 | 24.285 | 3.411 | 41.998 | 2/2 / 0/2 |
| [turbo-followup-ngram2](runs/turbo-followup-ngram2/result.json) / repetitive | passed | 2 | 184.403 | 21.804 | 21.754 | 2.714 | 22.771 | 0/2 / 0/2 |
| [turbo-followup-ngram4](runs/turbo-followup-ngram4/result.json) / prose | passed | 2 | 53.057 | 32.967 | 32.838 | 1.403 | 9.167 | — / — |
| [turbo-followup-ngram4](runs/turbo-followup-ngram4/result.json) / edit | passed | 2 | 290.474 | 28.881 | 28.851 | 3.467 | 35.943 | 2/2 / 0/2 |
| [turbo-followup-ngram4](runs/turbo-followup-ngram4/result.json) / repetitive | passed | 2 | 181.953 | 20.186 | 20.140 | 2.751 | 24.434 | 0/2 / 0/2 |
| [turbo-followup-no-cpu-fusion](runs/turbo-followup-no-cpu-fusion/result.json) / review | passed | 2 | 293.988 | 32.035 | 31.910 | 21.263 | 29.256 | — / — |
| [turbo-followup-no-cpu-fusion](runs/turbo-followup-no-cpu-fusion/result.json) / prose | passed | 2 | 56.436 | 32.298 | 32.172 | 1.313 | 9.239 | — / — |
| [turbo-followup-no-cpu-fusion](runs/turbo-followup-no-cpu-fusion/result.json) / edit | passed | 2 | 280.783 | 25.432 | 25.405 | 3.592 | 40.476 | 2/2 / 0/2 |
| [turbo-followup-overlap0](runs/turbo-followup-overlap0/result.json) / review | passed | 2 | 294.311 | 31.387 | 31.264 | 21.240 | 29.396 | — / — |
| [turbo-followup-overlap0](runs/turbo-followup-overlap0/result.json) / prose | passed | 2 | 56.039 | 31.737 | 31.613 | 1.322 | 9.388 | — / — |
| [turbo-followup-overlap0](runs/turbo-followup-overlap0/result.json) / edit | passed | 2 | 292.944 | 24.950 | 24.924 | 3.436 | 41.030 | 2/2 / 0/2 |
| [turbo-followup-overlap4](runs/turbo-followup-overlap4/result.json) / review | passed | 2 | 292.765 | 30.695 | 30.575 | 21.353 | 29.693 | — / — |
| [turbo-followup-overlap4](runs/turbo-followup-overlap4/result.json) / prose | passed | 2 | 55.827 | 31.205 | 31.083 | 1.327 | 9.530 | — / — |
| [turbo-followup-overlap4](runs/turbo-followup-overlap4/result.json) / edit | passed | 2 | 293.317 | 24.815 | 24.788 | 3.431 | 41.231 | 2/2 / 0/2 |
| [turbo-followup-overlap8](runs/turbo-followup-overlap8/result.json) / review | passed | 2 | 293.433 | 31.076 | 30.954 | 21.305 | 29.542 | — / — |
| [turbo-followup-overlap8](runs/turbo-followup-overlap8/result.json) / prose | passed | 2 | 55.626 | 31.206 | 31.084 | 1.332 | 9.535 | — / — |
| [turbo-followup-overlap8](runs/turbo-followup-overlap8/result.json) / edit | passed | 2 | 293.629 | 24.939 | 24.912 | 3.428 | 41.039 | 2/2 / 0/2 |
| [turbo-followup-reserve1024](runs/turbo-followup-reserve1024/result.json) / review | passed | 2 | 293.368 | 32.533 | 32.406 | 21.309 | 29.179 | — / — |
| [turbo-followup-reserve1024](runs/turbo-followup-reserve1024/result.json) / prose | passed | 2 | 56.258 | 33.852 | 33.720 | 1.317 | 8.879 | — / — |
| [turbo-followup-reserve1024](runs/turbo-followup-reserve1024/result.json) / edit | passed | 2 | 292.595 | 24.847 | 24.821 | 3.441 | 41.190 | 2/2 / 0/2 |
| [turbo-followup-threads6](runs/turbo-followup-threads6/result.json) / review | passed | 2 | 293.701 | 27.676 | 27.568 | 21.284 | 30.538 | — / — |
| [turbo-followup-threads6](runs/turbo-followup-threads6/result.json) / prose | passed | 2 | 56.076 | 27.898 | 27.789 | 1.321 | 10.497 | — / — |
| [turbo-followup-threads6](runs/turbo-followup-threads6/result.json) / edit | passed | 2 | 292.568 | 22.201 | 22.178 | 3.441 | 45.690 | 2/2 / 0/2 |
| [turbo-followup-threads8](runs/turbo-followup-threads8/result.json) / review | passed | 2 | 293.855 | 29.777 | 29.661 | 21.274 | 29.871 | — / — |
| [turbo-followup-threads8](runs/turbo-followup-threads8/result.json) / prose | passed | 2 | 55.221 | 29.499 | 29.384 | 1.342 | 10.022 | — / — |
| [turbo-followup-threads8](runs/turbo-followup-threads8/result.json) / edit | passed | 2 | 293.131 | 24.135 | 24.109 | 3.434 | 42.299 | 2/2 / 0/2 |
| [turbo-history-cache](runs/turbo-history-cache/result.json) / review | passed | 3 | 297.389 | 31.063 | 30.942 | 21.019 | 29.249 | — / — |
| [turbo-history-cache](runs/turbo-history-cache/result.json) / prose | passed | 3 | 56.657 | 31.494 | 31.371 | 1.307 | 9.436 | — / — |
| [turbo-history-cache](runs/turbo-history-cache/result.json) / edit | passed | 3 | 295.741 | 24.839 | 24.813 | 3.404 | 41.164 | 3/3 / 0/3 |
| [turbo-history-cache](runs/turbo-history-cache/result.json) / conversation | passed | 3 | 58.985 | 28.580 | 28.468 | 1.104 | 10.045 | — / — |
| [turbo-history-cacheoff](runs/turbo-history-cacheoff/result.json) / review | passed | 3 | 325.065 | 27.199 | 27.093 | 19.229 | 28.584 | — / — |
| [turbo-history-cacheoff](runs/turbo-history-cacheoff/result.json) / prose | passed | 3 | 62.714 | 28.153 | 28.044 | 1.181 | 10.274 | — / — |
| [turbo-history-cacheoff](runs/turbo-history-cacheoff/result.json) / edit | passed | 3 | 321.800 | 28.191 | 28.161 | 3.128 | 36.400 | 3/3 / 0/3 |
| [turbo-history-cacheoff](runs/turbo-history-cacheoff/result.json) / conversation | passed | 3 | 65.658 | 27.539 | 27.431 | 0.992 | 10.297 | — / — |
| [turbo-ple-lazy-only-cpu42](runs/turbo-ple-lazy-only-cpu42/result.json) / review | passed | 3 | 325.134 | 24.132 | 24.037 | 19.225 | 30.004 | — / — |
| [turbo-ple-lazy-only-cpu42](runs/turbo-ple-lazy-only-cpu42/result.json) / prose | passed | 3 | 62.776 | 25.095 | 24.997 | 1.180 | 11.381 | — / — |
| [turbo-ple-lazy-only-cpu42](runs/turbo-ple-lazy-only-cpu42/result.json) / edit | passed | 3 | 320.997 | 25.424 | 25.397 | 3.135 | 40.012 | 3/3 / 0/3 |
| [turbo-ple-lazy-only-cpu42](runs/turbo-ple-lazy-only-cpu42/result.json) / conversation | passed | 3 | 66.925 | 24.692 | 24.596 | 0.973 | 11.332 | — / — |
| [turbo-ple-lazy-reuse-cache6000](runs/turbo-ple-lazy-reuse-cache6000/result.json) / review | passed | 3 | 295.488 | 31.572 | 31.448 | 21.155 | 29.233 | — / — |
| [turbo-ple-lazy-reuse-cache6000](runs/turbo-ple-lazy-reuse-cache6000/result.json) / prose | passed | 3 | 55.417 | 30.800 | 30.680 | 1.337 | 9.640 | — / — |
| [turbo-ple-lazy-reuse-cache6000](runs/turbo-ple-lazy-reuse-cache6000/result.json) / edit | passed | 3 | 293.561 | 24.903 | 24.877 | 3.431 | 41.094 | 3/3 / 0/3 |
| [turbo-ple-lazy-reuse-cache6000](runs/turbo-ple-lazy-reuse-cache6000/result.json) / conversation | passed | 3 | 59.069 | 28.447 | 28.336 | 1.103 | 10.096 | — / — |
| [turbo-ple-lazy-reuse-cache6000-min256](runs/turbo-ple-lazy-reuse-cache6000-min256/result.json) / review | passed | 3 | 296.485 | 33.058 | 32.929 | 21.083 | 28.819 | — / — |
| [turbo-ple-lazy-reuse-cache6000-min256](runs/turbo-ple-lazy-reuse-cache6000-min256/result.json) / prose | passed | 3 | 55.752 | 30.242 | 30.124 | 1.329 | 9.767 | — / — |
| [turbo-ple-lazy-reuse-cache6000-min256](runs/turbo-ple-lazy-reuse-cache6000-min256/result.json) / edit | passed | 3 | 295.109 | 22.666 | 22.642 | 3.411 | 44.800 | 3/3 / 0/3 |
| [turbo-ple-lazy-reuse-cache6000-min256](runs/turbo-ple-lazy-reuse-cache6000-min256/result.json) / conversation | passed | 3 | 59.664 | 28.841 | 28.728 | 1.093 | 9.963 | — / — |
| [turbo-ple-lazy-reuse-cpu42](runs/turbo-ple-lazy-reuse-cpu42/result.json) / review | passed | 3 | 324.667 | 27.428 | 27.321 | 19.253 | 28.671 | — / — |
| [turbo-ple-lazy-reuse-cpu42](runs/turbo-ple-lazy-reuse-cpu42/result.json) / prose | passed | 3 | 62.400 | 28.548 | 28.436 | 1.187 | 10.141 | — / — |
| [turbo-ple-lazy-reuse-cpu42](runs/turbo-ple-lazy-reuse-cpu42/result.json) / edit | passed | 3 | 320.920 | 28.478 | 28.447 | 3.136 | 36.098 | 3/3 / 0/3 |
| [turbo-ple-lazy-reuse-cpu42](runs/turbo-ple-lazy-reuse-cpu42/result.json) / conversation | passed | 3 | 58.657 | 27.474 | 27.367 | 1.110 | 10.428 | — / — |
| [turbo-ple-lazy-reuse-cpu48](runs/turbo-ple-lazy-reuse-cpu48/result.json) / review | passed | 3 | 295.858 | 24.915 | 24.818 | 21.128 | 31.403 | — / — |
| [turbo-ple-lazy-reuse-cpu48](runs/turbo-ple-lazy-reuse-cpu48/result.json) / prose | passed | 3 | 56.891 | 25.828 | 25.727 | 1.303 | 11.213 | — / — |
| [turbo-ple-lazy-reuse-cpu48](runs/turbo-ple-lazy-reuse-cpu48/result.json) / edit | passed | 3 | 293.600 | 25.697 | 25.669 | 3.429 | 39.918 | 3/3 / 0/3 |
| [turbo-ple-lazy-reuse-cpu48](runs/turbo-ple-lazy-reuse-cpu48/result.json) / conversation | passed | 3 | 59.193 | 25.382 | 25.283 | 1.100 | 11.188 | — / — |
| [turbo-stock-cpu42-clean](runs/turbo-stock-cpu42-clean/result.json) / review | passed | 3 | 321.037 | 20.934 | 20.853 | 19.470 | 31.983 | — / — |
| [turbo-stock-cpu42-clean](runs/turbo-stock-cpu42-clean/result.json) / prose | passed | 3 | 59.998 | 25.244 | 25.146 | 1.234 | 11.372 | — / — |
| [turbo-stock-cpu42-clean](runs/turbo-stock-cpu42-clean/result.json) / edit | passed | 3 | 304.166 | 24.616 | 24.590 | 3.309 | 41.329 | 3/3 / 0/3 |
| [turbo-stock-cpu42-clean](runs/turbo-stock-cpu42-clean/result.json) / conversation | passed | 3 | 61.274 | 23.201 | 23.110 | 1.062 | 12.097 | — / — |
| [upstream-ada](runs/upstream-ada/result.json) / review | passed | 2 | 298.726 | 27.149 | 27.149 | 20.927 | 30.320 | — / — |
| [upstream-ada](runs/upstream-ada/result.json) / prose | passed | 2 | 56.902 | 28.402 | 28.402 | 1.306 | 10.293 | — / — |
| [upstream-ada](runs/upstream-ada/result.json) / edit | passed | 2 | 293.387 | 28.336 | 28.336 | 3.431 | 36.499 | 2/2 / 0/2 |

## Load and peak process resources

Load measurements reflect the observed OS page-cache state, not standardized cold boots. RSS includes file-backed mappings and is not equivalent to unreclaimable RAM. Total GPU memory includes the desktop. Review per-phase telemetry and smaps for interpretation.

| Run | Load s | Peak GPU MiB | Process RSS high-water MiB | Exit / errors |
|---|---:|---:|---:|---|
| [baseline-01](runs/baseline-01/launch.json) (passed) | 85.443 | 14753.000 | 41834.109 | 0  |
| [batch2048](runs/batch2048/launch.json) (failed) | — | 11509.000 | 44842.074 | 1 Server exited during load: 1 |
| [batch2048-cpu44](runs/batch2048-cpu44/launch.json) (passed) | 3.652 | 14507.000 | 44842.242 | 0  |
| [bulkcopy-base-0](runs/bulkcopy-base-0/launch.json) (passed) | 54.959 | 14346.000 | 44920.973 | 0  |
| [bulkcopy-base-1](runs/bulkcopy-base-1/launch.json) (passed) | 3.371 | 14230.000 | 44932.520 | 0  |
| [bulkcopy-bulk-0](runs/bulkcopy-bulk-0/launch.json) (passed) | 3.346 | 14368.000 | 44918.262 | 0  |
| [bulkcopy-bulk-1](runs/bulkcopy-bulk-1/launch.json) (passed) | 3.210 | 14229.000 | 44847.980 | 0  |
| [cold-ple-0-original-native](runs/cold-ple-0-original-native/launch.json) (passed) | 3.527 | 14747.000 | 44840.230 | 0  |
| [cold-ple-0-original-ple-parallel](runs/cold-ple-0-original-ple-parallel/launch.json) (passed) | 3.611 | 14747.000 | 44840.105 | 0  |
| [cold-ple-1-original-native](runs/cold-ple-1-original-native/launch.json) (passed) | 3.698 | 14747.000 | 44840.129 | 0  |
| [cold-ple-1-original-ple-parallel](runs/cold-ple-1-original-ple-parallel/launch.json) (passed) | 3.591 | 14747.000 | 44840.074 | 0  |
| [cold-ple-2-original-native](runs/cold-ple-2-original-native/launch.json) (passed) | 3.504 | 14747.000 | 44840.117 | 0  |
| [cold-ple-2-original-ple-parallel](runs/cold-ple-2-original-ple-parallel/launch.json) (passed) | 3.620 | 14747.000 | 44840.156 | 0  |
| [cow-bulkcopy-base-0](runs/cow-bulkcopy-base-0/launch.json) (passed) | 23.755 | 14420.000 | 44788.812 | 0  |
| [cow-bulkcopy-base-1](runs/cow-bulkcopy-base-1/launch.json) (passed) | 71.679 | 14306.000 | 43693.418 | 0  |
| [cow-bulkcopy-bulk-0](runs/cow-bulkcopy-bulk-0/launch.json) (passed) | 67.060 | 14329.000 | 44788.145 | 0  |
| [cow-bulkcopy-bulk-1](runs/cow-bulkcopy-bulk-1/launch.json) (passed) | 66.439 | 14364.000 | 43750.461 | 0  |
| [cow-prefault-0-0](runs/cow-prefault-0-0/launch.json) (passed) | 22.931 | 14216.000 | 44847.438 | 0  |
| [cow-prefault-0-1](runs/cow-prefault-0-1/launch.json) (passed) | 20.627 | 14168.000 | 44797.219 | 0  |
| [cow-prefault-0-2](runs/cow-prefault-0-2/launch.json) (passed) | 20.485 | 14194.000 | 44787.941 | 0  |
| [cow-prefault-12-0](runs/cow-prefault-12-0/launch.json) (passed) | 21.109 | 14220.000 | 44797.246 | 0  |
| [cow-prefault-12-1](runs/cow-prefault-12-1/launch.json) (passed) | 21.092 | 14235.000 | 44797.074 | 0  |
| [cow-prefault-12-2](runs/cow-prefault-12-2/launch.json) (passed) | 22.800 | 14234.000 | 44788.094 | 0  |
| [cow-prefault-6-0](runs/cow-prefault-6-0/launch.json) (passed) | 23.095 | 14206.000 | 44847.141 | 0  |
| [cow-prefault-6-1](runs/cow-prefault-6-1/launch.json) (passed) | 21.906 | 14178.000 | 44787.969 | 0  |
| [cow-prefault-6-2](runs/cow-prefault-6-2/launch.json) (passed) | 22.631 | 14186.000 | 44788.133 | 0  |
| [cpu-moe41](runs/cpu-moe41/launch.json) (passed) | 3.710 | 15529.000 | 44842.121 | 0  |
| [cuda-host-controlled-load-0](runs/cuda-host-controlled-load-0/launch.json) (passed) | 68.213 | 14210.000 | 35555.426 | 0  |
| [cuda-host-controlled-load-1](runs/cuda-host-controlled-load-1/launch.json) (passed) | 63.705 | 14194.000 | 35553.398 | 0  |
| [diagnostic-original-cupti-perf](runs/diagnostic-original-cupti-perf/launch.json) (instrumented) | 3.759 | 14761.000 | 44856.691 | 0  |
| [final-candidate-0](runs/final-candidate-0/launch.json) (passed) | 23.878 | 14438.000 | 44789.395 | 0  |
| [final-candidate-1](runs/final-candidate-1/launch.json) (passed) | 71.630 | 14439.000 | 43772.355 | 0  |
| [final-config-only-0](runs/final-config-only-0/launch.json) (passed) | 3.792 | 14658.000 | 44790.371 | 0  |
| [final-config-only-1](runs/final-config-only-1/launch.json) (passed) | 48.128 | 14643.000 | 43560.176 | 0  |
| [final-long80k-candidate-0](runs/final-long80k-candidate-0/launch.json) (passed) | 21.511 | 14255.000 | 44788.438 | 0  |
| [final-long80k-original-0](runs/final-long80k-original-0/launch.json) (passed) | 3.625 | 14499.000 | 44790.414 | 0  |
| [final-original-0](runs/final-original-0/launch.json) (passed) | 3.774 | 14635.000 | 44789.773 | 0  |
| [final-original-1](runs/final-original-1/launch.json) (passed) | 3.579 | 14596.000 | 44790.457 | 0  |
| [long80k-base-0](runs/long80k-base-0/launch.json) (passed) | 3.380 | 14243.000 | 44833.516 | 0  |
| [long80k-base-1](runs/long80k-base-1/launch.json) (passed) | 3.405 | 14310.000 | 44833.492 | 0  |
| [long80k-candidate-0](runs/long80k-candidate-0/launch.json) (passed) | 3.391 | 14248.000 | 44833.484 | 0  |
| [long80k-candidate-1](runs/long80k-candidate-1/launch.json) (passed) | 3.415 | 14220.000 | 44833.496 | 0  |
| [long80k-original-0](runs/long80k-original-0/launch.json) (passed) | 3.352 | 14402.000 | 44838.367 | 0  |
| [long80k-original-1](runs/long80k-original-1/launch.json) (passed) | 3.417 | 14604.000 | 44838.340 | 0  |
| [maskskip-affected-review-base-a](runs/maskskip-affected-review-base-a/launch.json) (passed) | 3.380 | 14305.000 | 44834.594 | 0  |
| [maskskip-affected-review-base-b](runs/maskskip-affected-review-base-b/launch.json) (passed) | 3.421 | 14243.000 | 44834.574 | 0  |
| [maskskip-affected-review-candidate](runs/maskskip-affected-review-candidate/launch.json) (passed) | 3.377 | 14304.000 | 44834.535 | 0  |
| [maskskip-base-real](runs/maskskip-base-real/launch.json) (passed) | 3.469 | 14687.000 | 45399.289 | 0  |
| [maskskip-base-real-resume1](runs/maskskip-base-real-resume1/launch.json) (passed) | 86.630 | 14385.000 | 45006.027 | 0  |
| [maskskip-candidate-real-resume1](runs/maskskip-candidate-real-resume1/launch.json) (passed) | 3.402 | 14389.000 | 45069.023 | 0  |
| [mtp-control-cpu46](runs/mtp-control-cpu46/launch.json) (passed) | 3.300 | 11012.000 | 44805.727 | 0  |
| [mtp-q8-cpu46-max2](runs/mtp-q8-cpu46-max2/launch.json) (passed) | 9.372 | 14588.000 | 44803.051 | 0  |
| [mtp-q8-cpu46-max4](runs/mtp-q8-cpu46-max4/launch.json) (passed) | 6.271 | 14820.000 | 44802.055 | 0  |
| [native-original](runs/native-original/launch.json) (passed) | 61.362 | 14749.000 | 42369.254 | 0  |
| [new-ada](runs/new-ada/launch.json) (passed) | 3.742 | 14804.000 | 44803.488 | 0  |
| [new-cow-b2048-cpu44](runs/new-cow-b2048-cpu44/launch.json) (passed) | 26.987 | 14780.000 | 44790.062 | 0  |
| [new-cow-pinned](runs/new-cow-pinned/launch.json) (passed) | 26.260 | 14763.000 | 44839.133 | 0  |
| [new-cpu-flat](runs/new-cpu-flat/launch.json) (excluded) | 55.380 | 14737.000 | 44011.762 | 0  |
| [new-cpu-flat-verified](runs/new-cpu-flat-verified/launch.json) (passed) | 3.731 | 14477.000 | 44802.742 | 0  |
| [new-direct-registered](runs/new-direct-registered/launch.json) (passed) | 3.755 | 14478.000 | 44839.137 | 0  |
| [new-graph-opt](runs/new-graph-opt/launch.json) (passed) | 3.747 | 14477.000 | 44805.312 | 0  |
| [new-local-control](runs/new-local-control/launch.json) (passed) | 3.768 | 14469.000 | 44839.078 | 0  |
| [new-no-graphs](runs/new-no-graphs/launch.json) (passed) | 3.806 | 14723.000 | 44806.203 | 0  |
| [new-noomp-poll0](runs/new-noomp-poll0/launch.json) (passed) | 3.698 | 14591.000 | 44839.117 | 0  |
| [new-noomp-registered](runs/new-noomp-registered/launch.json) (passed) | 3.497 | 14610.000 | 44839.090 | 0  |
| [new-omp-passive](runs/new-omp-passive/launch.json) (passed) | 3.532 | 14591.000 | 44839.082 | 0  |
| [new-pool-efficient64-stage](runs/new-pool-efficient64-stage/launch.json) (passed) | 3.221 | 13812.000 | 46596.168 | 0  |
| [new-pool-efficient80-stage](runs/new-pool-efficient80-stage/launch.json) (passed) | 3.272 | 15108.000 | 46622.488 | 0  |
| [new-pool32-cpu48](runs/new-pool32-cpu48/launch.json) (passed) | 3.162 | 11454.000 | 44839.129 | 0  |
| [new-pool64-pinned](runs/new-pool64-pinned/launch.json) (passed) | 97.172 | 14253.000 | 40254.605 | 0  |
| [new-pool64-stage](runs/new-pool64-stage/launch.json) (passed) | 3.328 | 14239.000 | 44836.953 | 0  |
| [new-pool80-stage](runs/new-pool80-stage/launch.json) (passed) | 3.387 | 15384.000 | 44837.648 | 0  |
| [new-registered](runs/new-registered/launch.json) (passed) | 3.679 | 14658.000 | 44839.125 | 0  |
| [new-rows2](runs/new-rows2/launch.json) (passed) | 3.459 | 14478.000 | 44839.125 | 0  |
| [new-stage-b1536-cpu42](runs/new-stage-b1536-cpu42/launch.json) (passed) | 3.914 | 15755.000 | 44790.969 | 0  |
| [new-stage-b2048-cpu44](runs/new-stage-b2048-cpu44/launch.json) (passed) | 60.577 | 14731.000 | 40941.977 | 0  |
| [new-stage-b4096-cpu48](runs/new-stage-b4096-cpu48/launch.json) (passed) | 3.607 | 14304.000 | 45644.809 | 0  |
| [new-stage-original-placement](runs/new-stage-original-placement/launch.json) (passed) | 48.979 | 14849.000 | 42856.164 | 0  |
| [newer-pinned](runs/newer-pinned/launch.json) (passed) | 76.832 | 14535.000 | 36604.414 | 0  |
| [newer-stock](runs/newer-stock/launch.json) (passed) | 3.629 | 14469.000 | 44843.062 | 0  |
| [ngram-adaptive-adaptive16-0](runs/ngram-adaptive-adaptive16-0/launch.json) (passed) | 3.327 | 14335.000 | 44846.898 | 0  |
| [ngram-adaptive-adaptive16-1](runs/ngram-adaptive-adaptive16-1/launch.json) (passed) | 3.414 | 14409.000 | 44844.875 | 0  |
| [ngram-adaptive-fixed16-0](runs/ngram-adaptive-fixed16-0/launch.json) (passed) | 3.411 | 14335.000 | 44846.715 | 0  |
| [ngram-adaptive-fixed16-1](runs/ngram-adaptive-fixed16-1/launch.json) (passed) | 3.494 | 14371.000 | 44846.855 | 0  |
| [ngram-adaptive-none-0](runs/ngram-adaptive-none-0/launch.json) (passed) | 3.451 | 14263.000 | 44846.793 | 0  |
| [ngram-adaptive-none-1](runs/ngram-adaptive-none-1/launch.json) (passed) | 3.214 | 14313.000 | 44844.793 | 0  |
| [ngram-ccd-t12-0](runs/ngram-ccd-t12-0/launch.json) (passed) | 3.318 | 14610.000 | 44851.398 | 0  |
| [ngram-ccd-t12-1](runs/ngram-ccd-t12-1/launch.json) (passed) | 3.287 | 14594.000 | 44852.488 | 0  |
| [ngram-ccd-t6-0](runs/ngram-ccd-t6-0/launch.json) (passed) | 3.485 | 14612.000 | 44851.199 | 0  |
| [ngram-ccd-t6-1](runs/ngram-ccd-t6-1/launch.json) (passed) | 3.539 | 14633.000 | 44851.312 | 0  |
| [ngram-rs-4-enabled0-0](runs/ngram-rs-4-enabled0-0/launch.json) (passed) | 3.318 | 12435.000 | 44846.742 | 0  |
| [ngram-rs-4-enabled0-1](runs/ngram-rs-4-enabled0-1/launch.json) (passed) | 3.352 | 12421.000 | 44844.773 | 0  |
| [ngram-rs-4-enabled1-0](runs/ngram-rs-4-enabled1-0/launch.json) (passed) | 3.241 | 12899.000 | 44844.719 | 0  |
| [ngram-rs-4-enabled1-1](runs/ngram-rs-4-enabled1-1/launch.json) (passed) | 3.294 | 12886.000 | 44846.758 | 0  |
| [ngram-rs-8-enabled0-0](runs/ngram-rs-8-enabled0-0/launch.json) (passed) | 3.216 | 12425.000 | 44846.703 | 0  |
| [ngram-rs-8-enabled0-1](runs/ngram-rs-8-enabled0-1/launch.json) (passed) | 3.380 | 12405.000 | 44846.766 | 0  |
| [ngram-rs-8-enabled1-0](runs/ngram-rs-8-enabled1-0/launch.json) (passed) | 3.214 | 13359.000 | 44846.707 | 0  |
| [ngram-rs-8-enabled1-1](runs/ngram-rs-8-enabled1-1/launch.json) (passed) | 3.373 | 13359.000 | 44846.703 | 0  |
| [original-after-turbo](runs/original-after-turbo/launch.json) (passed) | 3.742 | 14759.000 | 44805.098 | 0  |
| [original-api](runs/original-api/launch.json) (passed) | 3.601 | 14653.000 | 44842.125 | 0  |
| [original-controlled-load-0](runs/original-controlled-load-0/launch.json) (passed) | 3.134 | 14396.000 | 44789.770 | 0  |
| [original-controlled-load-1](runs/original-controlled-load-1/launch.json) (passed) | 2.892 | 14390.000 | 44789.613 | 0  |
| [original-threads24](runs/original-threads24/launch.json) (passed) | 3.672 | 14758.000 | 44838.473 | 0  |
| [parallel-original](runs/parallel-original/launch.json) (passed) | 3.620 | 14749.000 | 44839.566 | 0  |
| [place-down12](runs/place-down12/launch.json) (passed) | 3.472 | 14393.000 | 44805.648 | 0  |
| [place-gateup14](runs/place-gateup14/launch.json) (passed) | 3.524 | 14212.000 | 44805.785 | 0  |
| [place-gateup16](runs/place-gateup16/launch.json) (passed) | 3.501 | 14897.000 | 44805.789 | 0  |
| [place-mid7](runs/place-mid7/launch.json) (passed) | 3.687 | 14812.000 | 44805.777 | 0  |
| [place-mid8](runs/place-mid8/launch.json) (passed) | 3.582 | 15499.000 | 44805.953 | 0  |
| [poll0](runs/poll0/launch.json) (passed) | 3.663 | 14753.000 | 44842.121 | 0  |
| [spec-original-0-edit-0](runs/spec-original-0-edit-0/launch.json) (passed) | 3.603 | 14751.000 | 44805.746 | 0  |
| [spec-original-0-edit-1](runs/spec-original-0-edit-1/launch.json) (passed) | 3.633 | 14950.000 | 44805.766 | 0  |
| [spec-original-0-prose-0](runs/spec-original-0-prose-0/launch.json) (passed) | 3.672 | 14635.000 | 44805.789 | 0  |
| [spec-original-0-prose-1](runs/spec-original-0-prose-1/launch.json) (passed) | 3.651 | 14832.000 | 44805.727 | 0  |
| [spec-original-0-reasoning-0](runs/spec-original-0-reasoning-0/launch.json) (passed) | 3.635 | 14639.000 | 44805.754 | 0  |
| [spec-original-0-reasoning-1](runs/spec-original-0-reasoning-1/launch.json) (passed) | 3.742 | 14836.000 | 44805.754 | 0  |
| [spec-original-0-repetitive-0](runs/spec-original-0-repetitive-0/launch.json) (passed) | 3.663 | 14687.000 | 44805.730 | 0  |
| [spec-original-0-repetitive-1](runs/spec-original-0-repetitive-1/launch.json) (passed) | 3.661 | 14884.000 | 44805.762 | 0  |
| [spec-original-16-edit-0](runs/spec-original-16-edit-0/launch.json) (passed) | 3.683 | 15001.000 | 44805.809 | 0  |
| [spec-original-16-edit-1](runs/spec-original-16-edit-1/launch.json) (passed) | 3.667 | 14994.000 | 44805.766 | 0  |
| [spec-original-16-prose-0](runs/spec-original-16-prose-0/launch.json) (passed) | 3.707 | 14838.000 | 44805.699 | 0  |
| [spec-original-16-prose-1](runs/spec-original-16-prose-1/launch.json) (passed) | 3.670 | 14838.000 | 44805.773 | 0  |
| [spec-original-16-reasoning-0](runs/spec-original-16-reasoning-0/launch.json) (passed) | 3.683 | 14868.000 | 44805.750 | 0  |
| [spec-original-16-reasoning-1](runs/spec-original-16-reasoning-1/launch.json) (passed) | 3.695 | 14854.000 | 44805.785 | 0  |
| [spec-original-16-repetitive-0](runs/spec-original-16-repetitive-0/launch.json) (passed) | 3.708 | 14982.000 | 44805.754 | 0  |
| [spec-original-16-repetitive-1](runs/spec-original-16-repetitive-1/launch.json) (passed) | 3.693 | 14890.000 | 44805.797 | 0  |
| [spec-original-4-edit-0](runs/spec-original-4-edit-0/launch.json) (passed) | 3.723 | 14777.000 | 44805.719 | 0  |
| [spec-original-4-edit-1](runs/spec-original-4-edit-1/launch.json) (passed) | 3.660 | 14976.000 | 44805.711 | 0  |
| [spec-original-4-prose-0](runs/spec-original-4-prose-0/launch.json) (passed) | 3.638 | 14635.000 | 44805.785 | 0  |
| [spec-original-4-prose-1](runs/spec-original-4-prose-1/launch.json) (passed) | 3.628 | 14834.000 | 44805.738 | 0  |
| [spec-original-4-reasoning-0](runs/spec-original-4-reasoning-0/launch.json) (passed) | 3.599 | 14639.000 | 44805.688 | 0  |
| [spec-original-4-reasoning-1](runs/spec-original-4-reasoning-1/launch.json) (passed) | 3.704 | 14838.000 | 44805.844 | 0  |
| [spec-original-4-repetitive-0](runs/spec-original-4-repetitive-0/launch.json) (passed) | 3.621 | 14713.000 | 44805.703 | 0  |
| [spec-original-4-repetitive-1](runs/spec-original-4-repetitive-1/launch.json) (passed) | 3.683 | 14912.000 | 44805.770 | 0  |
| [spec-original-8-edit-0](runs/spec-original-8-edit-0/launch.json) (passed) | 3.665 | 15027.000 | 44805.703 | 0  |
| [spec-original-8-edit-1](runs/spec-original-8-edit-1/launch.json) (passed) | 3.667 | 14988.000 | 44805.781 | 0  |
| [spec-original-8-prose-0](runs/spec-original-8-prose-0/launch.json) (passed) | 3.658 | 14811.000 | 44805.785 | 0  |
| [spec-original-8-prose-1](runs/spec-original-8-prose-1/launch.json) (passed) | 3.639 | 14870.000 | 44805.773 | 0  |
| [spec-original-8-reasoning-0](runs/spec-original-8-reasoning-0/launch.json) (passed) | 3.642 | 14835.000 | 44805.832 | 0  |
| [spec-original-8-reasoning-1](runs/spec-original-8-reasoning-1/launch.json) (passed) | 3.578 | 14838.000 | 44805.809 | 0  |
| [spec-original-8-repetitive-0](runs/spec-original-8-repetitive-0/launch.json) (passed) | 3.680 | 14945.000 | 44805.777 | 0  |
| [spec-original-8-repetitive-1](runs/spec-original-8-repetitive-1/launch.json) (passed) | 3.660 | 14920.000 | 44805.832 | 0  |
| [spec-thread-16-t12-0](runs/spec-thread-16-t12-0/launch.json) (passed) | 3.702 | 14950.000 | 44805.777 | 0  |
| [spec-thread-16-t12-1](runs/spec-thread-16-t12-1/launch.json) (passed) | 3.535 | 14965.000 | 44805.789 | 0  |
| [spec-thread-16-t24-0](runs/spec-thread-16-t24-0/launch.json) (passed) | 3.656 | 14965.000 | 44805.773 | 0  |
| [spec-thread-16-t24-1](runs/spec-thread-16-t24-1/launch.json) (passed) | 3.695 | 14965.000 | 44805.871 | 0  |
| [spec-thread-4-t12-0](runs/spec-thread-4-t12-0/launch.json) (passed) | 3.699 | 14974.000 | 44805.672 | 0  |
| [spec-thread-4-t12-1](runs/spec-thread-4-t12-1/launch.json) (passed) | 3.691 | 14994.000 | 44805.762 | 0  |
| [spec-thread-4-t24-0](runs/spec-thread-4-t24-0/launch.json) (passed) | 3.590 | 14974.000 | 44805.758 | 0  |
| [spec-thread-4-t24-1](runs/spec-thread-4-t24-1/launch.json) (passed) | 3.436 | 14951.000 | 44805.730 | 0  |
| [spec-thread-8-t12-0](runs/spec-thread-8-t12-0/launch.json) (passed) | 3.697 | 14787.000 | 44805.730 | 0  |
| [spec-thread-8-t12-1](runs/spec-thread-8-t12-1/launch.json) (passed) | 3.644 | 14959.000 | 44805.820 | 0  |
| [spec-thread-8-t24-0](runs/spec-thread-8-t24-0/launch.json) (passed) | 3.641 | 14948.000 | 44805.820 | 0  |
| [spec-thread-8-t24-1](runs/spec-thread-8-t24-1/launch.json) (passed) | 3.671 | 14959.000 | 44805.801 | 0  |
| [stage-slot64-repeat](runs/stage-slot64-repeat/launch.json) (passed) | 3.648 | 14372.000 | 44788.168 | 0  |
| [stage-slot8-repeat](runs/stage-slot8-repeat/launch.json) (passed) | 44.731 | 14293.000 | 42823.742 | 0  |
| [state-equivalence-fork-cacheoff](runs/state-equivalence-fork-cacheoff/launch.json) (passed) | 3.583 | 14320.000 | 44804.766 | 0  |
| [state-equivalence-original](runs/state-equivalence-original/launch.json) (passed) | 3.647 | 14762.000 | 44805.801 | 0  |
| [threads12-batch12](runs/threads12-batch12/launch.json) (passed) | 3.439 | 14753.000 | 44842.230 | 0  |
| [threads6-batch12](runs/threads6-batch12/launch.json) (passed) | 5.401 | 14753.000 | 44841.781 | 0  |
| [threads8-batch12](runs/threads8-batch12/launch.json) (passed) | 3.592 | 14753.000 | 44842.254 | 0  |
| [turbo-cache-4480-cpu48](runs/turbo-cache-4480-cpu48/launch.json) (passed) | 152.257 | 13220.000 | 47162.730 | 0  |
| [turbo-cache-6000-cpu48](runs/turbo-cache-6000-cpu48/launch.json) (passed) | 160.276 | 13135.000 | 48261.785 | 0  |
| [turbo-cache-6000-cpu48-min256](runs/turbo-cache-6000-cpu48-min256/launch.json) (passed) | 157.844 | 13284.000 | 55134.484 | 0  |
| [turbo-cache-off-cpu42](runs/turbo-cache-off-cpu42/launch.json) (excluded) | 159.284 | 14780.000 | 43335.227 | 0  |
| [turbo-cache-off-cpu48](runs/turbo-cache-off-cpu48/launch.json) (passed) | 152.836 | 9027.000 | 51841.391 | 0  |
| [turbo-combined-control](runs/turbo-combined-control/launch.json) (passed) | 3.191 | 14770.000 | 44834.641 | 0  |
| [turbo-combined-cpu42-cache1024](runs/turbo-combined-cpu42-cache1024/launch.json) (passed) | 3.357 | 15321.000 | 44847.852 | 0  |
| [turbo-combined-cpu44-cache3072](runs/turbo-combined-cpu44-cache3072/launch.json) (passed) | 3.367 | 15195.000 | 44845.867 | 0  |
| [turbo-combined-cpu46-cache4480](runs/turbo-combined-cpu46-cache4480/launch.json) (passed) | 3.045 | 15021.000 | 44845.891 | 0  |
| [turbo-combined-min256](runs/turbo-combined-min256/launch.json) (passed) | 3.159 | 14759.000 | 44842.891 | 0  |
| [turbo-combined-ngram16-t12](runs/turbo-combined-ngram16-t12/launch.json) (passed) | 2.893 | 14692.000 | 44849.102 | 0  |
| [turbo-combined-ngram4-t12](runs/turbo-combined-ngram4-t12/launch.json) (passed) | 3.137 | 14698.000 | 44847.855 | 0  |
| [turbo-combined-ngram4-t24](runs/turbo-combined-ngram4-t24/launch.json) (passed) | 3.026 | 14744.000 | 44847.879 | 0  |
| [turbo-cow-enabled0](runs/turbo-cow-enabled0/launch.json) (passed) | 2.382 | 14672.000 | 44789.902 | 0  |
| [turbo-cow-enabled1](runs/turbo-cow-enabled1/launch.json) (passed) | 20.416 | 14752.000 | 44786.719 | 0  |
| [turbo-cow-ngram16-t12](runs/turbo-cow-ngram16-t12/launch.json) (passed) | 18.852 | 14728.000 | 44789.105 | 0  |
| [turbo-flat-cache](runs/turbo-flat-cache/launch.json) (passed) | 3.093 | 13080.000 | 47589.480 | 0  |
| [turbo-followup-async-fill](runs/turbo-followup-async-fill/launch.json) (passed) | 3.030 | 13080.000 | 46642.332 | 0  |
| [turbo-followup-batch1536](runs/turbo-followup-batch1536/launch.json) (passed) | 3.109 | 14896.000 | 46742.121 | 0  |
| [turbo-followup-batch2048-cache4480](runs/turbo-followup-batch2048-cache4480/launch.json) (passed) | 3.096 | 13606.000 | 46830.398 | 0  |
| [turbo-followup-low-churn](runs/turbo-followup-low-churn/launch.json) (passed) | 3.077 | 13080.000 | 46629.031 | 0  |
| [turbo-followup-ngram2](runs/turbo-followup-ngram2/launch.json) (passed) | 3.060 | 13088.000 | 45162.359 | 0  |
| [turbo-followup-ngram4](runs/turbo-followup-ngram4/launch.json) (passed) | 3.050 | 13124.000 | 45187.586 | 0  |
| [turbo-followup-no-cpu-fusion](runs/turbo-followup-no-cpu-fusion/launch.json) (passed) | 3.096 | 13080.000 | 46623.754 | 0  |
| [turbo-followup-overlap0](runs/turbo-followup-overlap0/launch.json) (passed) | 3.067 | 13080.000 | 46614.172 | 0  |
| [turbo-followup-overlap4](runs/turbo-followup-overlap4/launch.json) (passed) | 3.058 | 13080.000 | 46633.023 | 0  |
| [turbo-followup-overlap8](runs/turbo-followup-overlap8/launch.json) (passed) | 3.072 | 13080.000 | 46610.398 | 0  |
| [turbo-followup-reserve1024](runs/turbo-followup-reserve1024/launch.json) (passed) | 3.090 | 14664.000 | 46632.168 | 0  |
| [turbo-followup-threads6](runs/turbo-followup-threads6/launch.json) (passed) | 3.132 | 13080.000 | 46604.594 | 0  |
| [turbo-followup-threads8](runs/turbo-followup-threads8/launch.json) (passed) | 3.072 | 13080.000 | 46613.516 | 0  |
| [turbo-history-cache](runs/turbo-history-cache/launch.json) (passed) | 2.876 | 13114.000 | 47608.438 | 0  |
| [turbo-history-cacheoff](runs/turbo-history-cacheoff/launch.json) (passed) | 3.464 | 14324.000 | 44805.680 | 0  |
| [turbo-ple-lazy-only-cpu42](runs/turbo-ple-lazy-only-cpu42/launch.json) (passed) | 9.239 | 14474.000 | 44787.773 | 0  |
| [turbo-ple-lazy-reuse-cache6000](runs/turbo-ple-lazy-reuse-cache6000/launch.json) (passed) | 3.055 | 13082.000 | 47713.199 | 0  |
| [turbo-ple-lazy-reuse-cache6000-min256](runs/turbo-ple-lazy-reuse-cache6000-min256/launch.json) (passed) | 3.037 | 13082.000 | 47758.754 | 0  |
| [turbo-ple-lazy-reuse-cpu42](runs/turbo-ple-lazy-reuse-cpu42/launch.json) (passed) | 3.477 | 14294.000 | 44789.859 | 0  |
| [turbo-ple-lazy-reuse-cpu48](runs/turbo-ple-lazy-reuse-cpu48/launch.json) (passed) | 3.064 | 8658.000 | 47503.141 | 0  |
| [turbo-stock-cpu42-clean](runs/turbo-stock-cpu42-clean/launch.json) (passed) | 157.098 | 14510.000 | 50717.508 | 0  |
| [upstream-ada](runs/upstream-ada/launch.json) (passed) | 3.716 | 14772.000 | 44803.176 | 0  |

## Exclusions

- `turbo-cache-off-cpu42`: Stale turbo-screen-done marker from an earlier failed-build dispatch admitted source-overlay compilation and cache unit testing while the stock run remained active. Exclude whole run from speed comparisons; retain raw evidence. Repeat as turbo-stock-cpu42-clean.
- `new-cpu-flat`: Intentionally interrupted before retrying the failed TurboQuant build command; no scored requests. See build-queue-interruption.md. Repeat is new-cpu-flat-verified.
- `maskskip-candidate-real`: Intentionally interrupted at user-requested OS-switch pause; preserve raw data and rerun under a fresh name.

Process status counts: excluded=2, failed=1, instrumented=1, passed=196.
All original source/runtime/model preservation checks and final recommendations remain separate from this generated catalog.
