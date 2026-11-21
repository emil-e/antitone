# SegFormer Roll WB model

`segformer-b0.onnx` is the CPU segmentation model for [Analyze](../../docs/roll-analysis.md#4-set-shared-photographic-wb). It is an ONNX opset 17 export of [KerasHub's SegFormer-B0 ADE20K 512 preset](https://huggingface.co/keras/segformer_b0_ade20k_512/tree/fca25aef87ab4226b80f5220e14f327e208bdf57). Keras publishes the preset weights under MIT and its model code under Apache 2.0; see the [pinned model card](KERAS-MODEL-CARD.md). Keras code is not shipped with Antitone.

| Artifact | SHA-256 |
| --- | --- |
| Original `model.weights.h5` | `f4cb2a42734c41e9534370119337b0e1f4c9b5b77e53d339a1ae8201bbce274a` |
| Original `task.weights.h5` | `1a9d6a6eb41586d4959123a59e5eb4956fb11f48224758bcc2c6cb3c37061281` |
| Embedded ONNX | `9f132942a45b822f73ae1ea8662a7581c1f456afac04f577e43ab4de94f81673` |

The graph expects normalized FP32 RGB `pixels` in NCHW `[1,3,512,512]` and returns pre-resize ADE20K logits in `[1,150,128,128]`. Preprocessing and site sampling live in [the classifier](../../src/editor/analysis/semantic.rs).

The 139 compared nonconstant parameter tensors are bit-identical after layout conversion to those in NVIDIA's B0 ADE20K checkpoint, including the classifier. That establishes shared parameter values; the release notice above is from the pinned Keras artifact, not inferred from the matching tensors.
