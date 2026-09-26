# Higher-precision alternative: separate model comparison

Status: remote headers inspected; download and real inference queued after the isolated source screens. No additional model has been downloaded yet.

The existing 28 local shards match all 28 SHA-256 hashes published by [AtomicChat](https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF) at revision `142262902a46f7daed19c79d0771534c8106ad59`. The local directory's `unsloth` component is therefore not sufficient provenance by itself. This comparison used the previously captured local hashes; it did not re-read the model during inference.

The same publisher offers a 4.27-bpw variant and reports substantially lower divergence from its BF16 reference than its 3.84-bpw variant. Those are publisher measurements, not independent quality validation here. The API's actual shard sizes total **88.034 GiB**, which takes precedence over approximate model-card totals for disk planning. About285 GiB was free before queuing the download. An80 GiB reserve is enforced by the download script.

HTTP range inspection of all33 GGUF headers found the same1,224 tensor names and identical dimensions. Changes are confined to140 expert tensor quantization types:

| Change | Tensor count |
|---|---:|
| IQ1_M → IQ2_S |68|
| IQ1_M → IQ3_S |4|
| IQ2_S → IQ3_S |20|
| MXFP4 → IQ4_NL |48|

The PLE table remains Q5_1; remaining tensor types are unchanged. This is a higher-precision candidate, not a proposal to quantize the existing model more aggressively. Equal shapes/types alone do not prove equal trained weights or quality; downloaded hashes, tokenizer compatibility and actual task checks remain required.

The performance hypothesis is specific: removing IQ1_M from the GPU prefill path may avoid its dequantization/cuBLAS fallback, and different CPU dot-product kernels may offset some of the additional weight bandwidth. The counterargument is also concrete: larger expert tensors increase RAM/PCIe traffic and reduce VRAM headroom. Only measurement can settle this.

The planned comparison uses a separate model directory and matched CPU44 placement, context90,000, Q8 K/V, batching and frozen prompts. Its results must be labeled **different higher-precision quantization**, never counted as a lossless same-file runtime improvement. No production profile is changed. Downloader verifies every remote SHA-256 and runs only after all timed inference exits.

Evidence: `atomic-model-catalog.json`, `alternative-427-remote-layout.json`, `alternative-427-assessment.json`. Reproduction: `inspect-remote-alternative.py`, `alternative-model-test.py` in the experiment root.
