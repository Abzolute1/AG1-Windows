---
license: other
license_name: qwen-community-1.0
license_link: https://huggingface.co/Qwen/Qwen3.8-Flash-Next/blob/main/LICENSE
base_model:
- Qwen/Qwen3.8-Flash-Next
base_model_relation: quantized
quantized_by: AtomicChat
pipeline_tag: text-generation
library_name: gguf
tags:
- atomic-chat
- qwen
- qwen3.8
- flash-next
- moe
- multimodal
- gguf
- imatrix
- quantized
- llama.cpp
---

# How to Run Qwen3.8-Flash-Next Locally
<p style="margin-top: 0; margin-bottom: 0;">
  <em>Built from Qwen's original weights with our own importance matrix. The <a href="https://huggingface.co/datasets/AtomicChat/calib-corpora">calibration corpora</a> behind our builds are public.</em>
</p>
<div style="display: flex; gap: 8px; align-items: center; margin-top: 10px; margin-bottom: 10px;">
  <a href="https://atomic.chat/?utm_source=huggingface&utm_medium=referral&utm_campaign=hf_qwen3_8_flash_next&utm_content=btn_atomic"><img src="https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF/resolve/main/btn_atomic.png" width="162" alt="Atomic Chat"></a>
  <a href="https://discord.gg/8wGSsvmg4V"><img src="https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF/resolve/main/btn_discord.png" width="119" alt="Discord"></a>
  <a href="https://github.com/AtomicBot-ai/Atomic-Chat"><img src="https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF/resolve/main/btn_github.png" width="115" alt="GitHub"></a>
</div>
<ul style="margin: 0 0 12px 0;">
  <li>Qwen3.8-Flash-Next is the first open-weight release of the architecture behind Qwen4.</li>
  <li>These GGUFs are self-quantized from Qwen's original weights with our own importance matrix, published alongside the quants.</li>
  <li>The quants are still uploading and need a llama.cpp build with Qwen3.8-Flash-Next support; Atomic Chat runs it as support ships.</li>
</ul>
<hr style="margin: 0 0 16px 0;">


## Running a 176B model on a 64 GB MacBook

Qwen3.8-Flash-Next has 177B parameters. Our 85 GB quant runs on an M5 Max with
64 GB of memory, with vision, at 36 tok/s. That is not a typo: the file is
larger than the machine's entire RAM.

It works because 39 GB of that file never enters memory at all.

```
memory breakdown [MiB]  |  total    free     self   model  context  compute
  - MTL0 (Apple M5 Max) |  57344 = 13217 + (44125 = 43720 +   256 +    148)
  - Host                |                   37279 = 37265 +     0 +     14
```

### Why this is possible

51B of the model's 177B parameters are not weights in the usual sense. They are
an **n-gram lookup table**. The model hashes the last three tokens, and that
hash points at 16 rows of 160 values each. Roughly 2.7 KB per token, read once
per forward pass, out of a 39 GB table.

That is a 1-in-13-million read ratio, at a deterministic address. At 36 tok/s it
comes to about 3 MB/s of random reads, and NVMe answers in under 100 µs against
a 28 ms per-token budget. Common n-grams stay in page cache anyway.

Compare that with the experts: they touch about 6B parameters per token,
gigabytes of traffic, and would be hopeless from disk. That is why ordinary
offloading fails when you run out of memory, and why this table is different.

> [!IMPORTANT]
> On Apple Silicon this only works if the table sits in **its own GGUF shard**.
> llama.cpp hands Metal the entire mmap'd region of any shard containing GPU
> tensors, so a table interleaved with weights gets wired along with them. The
> model then asks for more memory than the machine has, and the first decode
> dies with `kIOGPUCommandBufferCallbackErrorOutOfMemory`. Every quant here is
> split so that shard 2 holds nothing but the table.

## The quants

| Build | In memory | On SSD | Total | Mean KLD | Same top-1 | PPL ratio |
|---|---|---|---|---|---|---|
| `AD-3.84bpw-IQ4_XS-M64` | **45.8 GB** | 39.1 GB | 84.9 GB | 0.2277 | 82.68% | 1.102 |
| `AD-4.27bpw-Q4_K_M-M64` | 54.5 GB | 38.4 GB | 92.9 GB | **0.0842** | **89.49%** | 1.026 |
| `AD-5.00bpw-Q5_K_M-M64` | 56.1 GB | 54.4 GB | 110.5 GB | 0.0837 | 89.55% | 1.026 |

**`AD-4.27bpw` is the one to take.** It matches the 5.00bpw build within
measurement error while being 17.6 GB smaller, and leaves more headroom for
context. The 3.84bpw build is the one in the video; it is kept for machines
where every gigabyte of RAM counts.

"In memory" is what the GPU actually holds. The n-gram table is excluded because
it stays on SSD.

Everything was measured against **one reference, one corpus, one machine**: the
BF16 model's own logits over a held-out neutral set, 87 chunks at 4096 context,
BF16 PPL 4.0445 ± 0.0216. Other publishers' files were downloaded and re-measured
here rather than having their numbers copied, because figures taken against
different references are not comparable. Their builds keep the n-gram table
inside the weight shards, so the whole file has to be resident.

## Running them

```bash
sudo sysctl iogpu.wired_limit_mb=57344
```

```bash
llama-cli -m Qwen3.8-Flash-Next-AD-4.27bpw-Q4_K_M-M64-00001-of-00033.gguf \
  -ngl 99 -c 32768 --jinja -fit off
```

With vision:

```bash
llama-mtmd-cli -m Qwen3.8-Flash-Next-AD-4.27bpw-Q4_K_M-M64-00001-of-00033.gguf \
  --mmproj mmproj-Qwen3.8-Flash-Next-F16.gguf \
  -ngl 99 -c 32768 -b 512 --image-min-tokens 1024 --jinja -fit off
```

Three things matter here. Keep mmap **on**, that is what makes the table
pageable, so no `--load-mode none`. No `--override-tensor` is needed, the table
goes to the host by itself. And `-fit off` is required: llama.cpp's automatic
parameter fitting mis-sizes this architecture and fails to allocate.

Measured on that Mac: **pp512 517.9 tok/s, tg128 36.0 tok/s**.

## The importance matrix

Published here as `imatrix.gguf`, computed on BF16 weights, never on a quantized
proxy.

| | |
|---|---|
| Chunks | 4000 at 512 context |
| Corpus | 4,967,044 tokens, 3,004 documents |
| Entries | 926 tensors |
| PPL over the calibration set | 4.6812 ± 0.0155 |
| Coverage gaps | `blk.0` at 98.83%, `blk.47` at 99.80%, everything else complete |
| Compute time | 3 h 8 min on 4× B200 |

The corpus is rendered through this model's own chat template, so calibration
matches inference byte for byte. Composition: agentic 24.7%, code 17.8%,
reasoning 14.8%, multilingual 13.8%, long context 11.9%, vocabulary sweep 9.9%,
structured 4.1%, graphics 3.0%. It is public in the
[calibration corpora](https://huggingface.co/datasets/AtomicChat/calib-corpora)
dataset.

The two remaining gaps are experts the router never selects on this corpus. Six
out of 24,576 in `blk.0`, one in `blk.47`. Going from 1200 to 4000 chunks closed
half of them; the rest do not close at any corpus size.

### What the matrix told us about the layout

![image](https://cdn-uploads.huggingface.co/production/uploads/6a54dea4f19f5386700504da/TGyW_LniyuJ-FxgwO9KrC.png)

Running `llama-imatrix --show-statistics` puts `attn_gate` at the top of every
position in the ranking. By layer, the energy concentrates in the **tail**:
layers 40, 46, 42, 41, 45 and 44 hold six of the seven highest values, and the
only representative from the head is `blk.0`. Layer 1 sits at 343, four times
lower than layer 0.

So the high-bit band in our recipe is asymmetric: blocks **0-3 and 40-47** get a
step up, instead of the symmetric band a naive ladder would use. Spending bits
evenly wastes them on layers 4 through 39, which do not need them.

One curiosity worth recording: `blk.0.attn_gate` has a maximum activation of
33.5 while every other layer sits between 3 and 8. The outlier reproduces at
both 1200 and 4000 chunks, so it is a property of the model rather than sampling
noise.

## Three traps specific to this architecture

**`moe_intermediate_size` is 640.** k-quants and i-quants need rows divisible by
256, and 640 is not, so `ffn_down_exps` (23% of the model) falls back silently:
ask for IQ2_XXS and you get IQ4_NL, ask for Q6_K and you get Q8_0. Only block-32
types work there and 4.25 bits is the floor for that group, however aggressive
the rest of the recipe is.

**The n-gram table gets no importance matrix.** It is a `GET_ROWS` tensor, so
`llama-imatrix` collects no statistics for it at any corpus size. Those 51.2B
parameters are quantized blind. It also has ncols 160, so it too takes only
block-32 types. We tested 6 bits against 8.5 on otherwise identical builds: mean
KLD moved by 0.0005, which is the size of the error bar. Six bits it is.

**`mxfp4` discards the importance matrix.** Its ggml implementation calls
`GGML_UNUSED(quant_weights)` outright. Using it for `ffn_down_exps` costs
calibration on another 23% of the model to save 0.25 bits per weight.

Fixing all three, and placing the band by activation statistics, cut mean KLD by
**63%** at the same file size: 0.2277 down to 0.0842, with top-1 up 6.8 points.

## Naming

Files are named by their measured bits per weight. A build whose expert tensors
are IQ1_M is not a 1-bit model when the n-gram table sits at 6 bits and
`ffn_down_exps` at 4.5; the real average is 3.84. The canonical type in the
filename is the closest standard type by that average, so tooling can still
detect it. For `AD-4.27bpw`:

| Group | Type | Share of file | Contribution |
|---|---|---|---|
| n-gram table | Q5_1 | 41% | 1.74 bpw |
| `ffn_gate/up_exps` | IQ2_S, IQ3_S at the band | 29% | 1.24 bpw |
| `ffn_down_exps` | IQ4_NL | 24% | 1.03 bpw |
| everything else | Q8_0 | 5% | 0.23 bpw |

## Reproducing

```bash
llama-quantize --imatrix imatrix.gguf \
  --tensor-type 'blk\.([0-3]|4[0-7])\.ffn_(gate|up)_exps=iq3_s' \
  --tensor-type 'ffn_down_exps=iq4_nl' \
  --tensor-type 'ffn_gate_exps=iq2_s' \
  --tensor-type 'ffn_up_exps=iq2_s' \
  --tensor-type 'per_layer_token_embd=q5_1' \
  Qwen3.8-Flash-Next-BF16.gguf out.gguf q8_0
```

```bash
llama-gguf-split --split --split-max-size 2G out.gguf split/prefix
```

The table is the fifth tensor in the file, so any small size limit isolates it
into shard 2. Needs a llama.cpp build with qwen4exp support
([PR #27742](https://github.com/ggml-org/llama.cpp/pull/27742)).

All KLD logs, the BF16 reference and both importance matrices are in the
[metrics repository](https://huggingface.co/datasets/AtomicChat/Qwen3.8-Flash-Next-GGUF-metrics).

<img src="https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF/resolve/main/hero.png" alt="Qwen3.8-Flash-Next architecture" style="width:560px; max-width:100%; height:auto;"/>

*Qwen3.8-Flash-Next architecture (Qwen).*

## Highlights

- **125B total with 6B active** sparse MoE (512 experts, 10 routed + 1 shared), plus a 51B n-gram embedding and a 4B MTP layer. An experimental preview of the architecture behind Qwen4.
- **Hybrid attention with QSA**: Gated DeltaNet paired with Qwen Sparse Attention, which operates at the micro-block level rather than per token to cut long-context latency for agentic workloads.
- **Gated Residual**: a data-dependent read gate plus a per-branch scalar write gate over widened residual streams, for finer expressiveness at low inference overhead.
- **N-gram Embedding**: 20M bigram/trigram embeddings indexed at layer 2, a compute-light axis for parameter scaling that offloads well on memory-constrained accelerators.
- **262,144-token context**, extensible up to 1,000,000 tokens with RoPE scaling.
- **Natively multimodal** (causal language model with a vision encoder, image-text-to-text). These GGUF quants cover the text path.
- **Frontier coding and agentic scores** (Qwen-reported): LiveCodeBench v6 91.9, GPQA Diamond 91.7, SWE-bench Multilingual 81.0, CoWorkBench 73.9.
- **Full imatrix quantization** with our public [calibration corpora](https://huggingface.co/datasets/AtomicChat/calib-corpora).

> [!NOTE]
> These GGUFs are **self-quantized from the original weights**, not a repack. The importance matrix keeps low-bit quants closer to the full-precision model.

> [!IMPORTANT]
> Always pass `--jinja` so the **Qwen3.8-Flash-Next chat template** is applied. Without it the model can emit malformed turns.

## Model Overview

| Property | Value |
|---|---|
| Base model | `Qwen/Qwen3.8-Flash-Next` |
| Type | Causal language model with a vision encoder (image-text-to-text) |
| Total / active parameters | 125B total / 6B active, plus 51B n-gram embedding and a 4B MTP layer |
| Layers | 48. Hidden layout: 12 x (3 x (Gated DeltaNet then MoE) then 1 x (Qwen Sparse Attention then MoE)) |
| Experts | 512 experts, 10 routed + 1 shared activated |
| Attention | Hybrid: Gated DeltaNet (linear) and Qwen Sparse Attention (micro-block sparse); Gated Residual over widened residual streams |
| Context length | 262,144 native, extensible up to 1,000,000 |
| This repo | GGUF quants (imatrix), text path. The importance matrix we built is published here too. |

<img src="https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF/resolve/main/benchmark.png" alt="Qwen3.8-Flash-Next benchmark scores" style="width:100%; max-width:900px;"/>

Scores are Qwen's published results for the base `Qwen/Qwen3.8-Flash-Next`. Quantization preserves the large majority of this; `Q4_K_M` and up sit within a point or two of full precision.


## Choosing a quant

| Quant | Size | Notes |
|---|---|---|
| `IQ2_M` | — | Smallest usable. Aggressive low-bit for memory-constrained boxes. |
| `IQ3_M` | — | Beats Q3 at similar size thanks to imatrix. Best low-RAM pick. |
| **`Q4_K_M`** | — | **Recommended default. Best balance of size, speed and quality.** |
| **`UD-Q4_K_XL`** | — | **Dynamic. Embeddings and output kept at Q8_0 for higher quality at a Q4 footprint.** |
| `Q6_K` | — | Near lossless. |
| `Q8_0` | — | Effectively lossless, reference quality. |

> [!TIP]
> Sizes fill in once the quants finish uploading. Pick the largest file that fits your (V)RAM with room for context.

## Get started

> [!NOTE]
> Qwen3.8-Flash-Next is a brand-new Qwen4-preview architecture (Gated DeltaNet, Qwen Sparse Attention, n-gram embedding). The quants in this repo are still uploading, and running them needs a `llama.cpp` build that has landed Qwen3.8-Flash-Next support. Until then, [Atomic Chat](https://atomic.chat) is the easiest way to run it as support ships.

Run Qwen3.8-Flash-Next locally with:

- **[Atomic Chat](https://atomic.chat):** the easiest path. Open the app, search `AtomicChat/Qwen3.8-Flash-Next-GGUF`, pick a quant, hit **Use this model**.
- **llama.cpp:** `llama-server -hf AtomicChat/Qwen3.8-Flash-Next-GGUF:Q4_K_M --jinja -c 8192`
- **Ollama:** `ollama run hf.co/AtomicChat/Qwen3.8-Flash-Next-GGUF:Q4_K_M`
- **LM Studio / Jan:** search the repo id, download any quant.

## Best practices

| Parameter | Value |
|---|---|
| temperature | 1.0 |
| top_p | 0.95 |
| top_k | 20 |
| min_p | 0.0 |

Qwen's recommended thinking-mode settings. For non-thinking (instruct) use `temperature=0.7`, `top_p=0.80`, `top_k=20`, `presence_penalty=1.5`. Allocate generous output length for agentic tasks.

## Run in llama.cpp

```bash
git clone https://github.com/ggerganov/llama.cpp
cmake llama.cpp -B llama.cpp/build -DBUILD_SHARED_LIBS=OFF -DGGML_CUDA=ON
cmake --build llama.cpp/build --config Release -j --target llama-cli llama-server
```

```bash
./llama.cpp/build/bin/llama-server \
    -hf AtomicChat/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL \
    --jinja -ngl 99 -c 8192 -fa on
```

## How these were made

1. Download `Qwen/Qwen3.8-Flash-Next` (original weights).
2. Convert to GGUF with a [llama.cpp](https://github.com/ggerganov/llama.cpp) build that supports the Qwen3.8-Flash-Next architecture (Gated DeltaNet, Qwen Sparse Attention, n-gram embedding).
3. Build an importance matrix over our public [calibration corpora](https://huggingface.co/datasets/AtomicChat/calib-corpora).
4. Quantize the ladder with `--imatrix`; `UD-Q4_K_XL` additionally pins the token-embedding and output tensors to `Q8_0`.

## License

Released by Qwen under the Qwen Community License 1.0. Quantized by Atomic Chat.

