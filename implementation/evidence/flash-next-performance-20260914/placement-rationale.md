# Quantization-aware placement candidates

Original whole-layer placement spends GPU expert memory on the last six layers, whose gate/up matrices use IQ2_S. Middle-layer gate/up matrices use IQ1_M and occupy less space. The isolated placement screen tests seven/eight middle layers for the same approximate expert VRAM budget; it also tests gate/up-only placement across14/16 layers and down-only placement across12 layers.

Technical hypotheses: IQ1_M CPU dequantization is visible in perf; putting more of these matrices on the GPU may reduce CPU work per byte of VRAM. Grouping gate/up on GPU can avoid two CPU expert operations for each selected layer. Conversely, MXFP4 down projection has substantial CPU cost and may benefit from dedicated placement. None changes weights, active expert counts, routing or context. CPU versus CUDA quantized execution may differ numerically, so exact-edit and broader quality comparisons remain required. Allocation failures are retained rather than reducing context or cache precision to make a candidate fit.

The explicit GPU patterns precede the catch-all CPU expert pattern; the original --n-cpu-moe override is removed. All other tensors retain the original automatic GPU policy. See placement-configs.json in the experiment root for exact argv overrides.
