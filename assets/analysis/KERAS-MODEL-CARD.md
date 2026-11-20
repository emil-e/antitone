---
library_name: keras-hub
---
### Model Overview
The SegFormer model was proposed in SegFormer: Simple and Efficient Design for Semantic Segmentation with Transformers by Enze Xie, Wenhai Wang, Zhiding Yu, Anima Anandkumar, Jose M. Alvarez, Ping Luo. The model consists of a hierarchical Transformer encoder and a lightweight all-MLP decode head to achieve great results on image segmentation benchmarks such as ADE20K and Cityscapes.

`
Weights are released under the [MIT License](https://opensource.org/license/mit). Keras model code is released under the [Apache 2 License](https://github.com/keras-team/keras-hub/blob/master/LICENSE).

## Links

* [SegFormer Quickstart Notebook](https://www.kaggle.com/code/laxmareddypatlolla/segformer-quickstart-notebook)
* [SegFormer API Documentation](https://keras.io/keras_hub/api/models/segformer/)
* [SegFormer Model Card](https://huggingface.co/docs/transformers/en/model_doc/segformer)
* [KerasHub Beginner Guide](https://keras.io/guides/keras_hub/getting_started/)
* [KerasHub Model Publishing Guide](https://keras.io/guides/keras_hub/upload/)

## Installation

Keras and KerasHub can be installed with:

```
pip install -U -q keras-hub
pip install -U -q keras
```

Jax, TensorFlow, and Torch come preinstalled in Kaggle Notebooks. For instructions on installing them in another environment see the [Keras Getting Started](https://keras.io/getting_started/) page.

## Presets

The following model checkpoints are provided by the Keras team. Full code examples for each are available below.
| Preset name           | Parameters | Description                                       |
|-----------------------|------------|---------------------------------------------------|
| segformer_b0_ade20k_512 | 3.72M      | SegFormer model with MiTB0 backbone fine-tuned on ADE20k in 512x512 resolution. |
| segformer_b1_ade20k_512          | 13.68M     | SegFormer model with MiTB0 backbone fine-tuned on ADE20k in 512x512 resolution. |
| segformer_b2_ade20k_512        | 24.73M    | SegFormer model with MiTB2 backbone fine-tuned on ADE20k in 512x512 resolution.             |
| segformer_b3_ade20k_512        | 44.60M     | SegFormer model with MiTB2 backbone fine-tuned on ADE20k in 512x512 resolution.             |
| segformer_b4_ade20k_512       | 61.37M     |SegFormer model with MiTB2 backbone fine-tuned on ADE20k in 512x512 resolution.               |
| segformer_b5_ade20k_640    | 81.97M       | SegFormer model with MiTB5 backbone fine-tuned on ADE20k in 640x640 resolution.             |
| segformer_b0_cityscapes_1024     | 3.72M       | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.              |
| segformer_b1_cityscapes_1024     | 13.68M       | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.               |
| segformer_b2_cityscapes_1024     | 24.73M        | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.              |
| segformer_b3_cityscapes_1024     | 44.60M        | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.              |
| segformer_b4_cityscapes_1024     | 61.37M          | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.              |
| segformer_b5_cityscapes_1024     | 81.97M        | SegFormer model with MiTB0 backbone fine-tuned on Cityscapes in 1024x1024 resolution.              |

## Example Usage
```python
import keras_cv
import  keras
import numpy as np
```



Using the class with a `backbone`:

```python
import tensorflow as tf
import keras_cv

images = np.ones(shape=(1, 96, 96, 3))
labels = np.zeros(shape=(1, 96, 96, 1))
backbone = keras_cv.models.MiTBackbone.from_preset("segformer_b0_ade20k_512")
model = keras_cv.models.segmentation.SegFormer(
    num_classes=1, backbone=backbone,
)

# Evaluate model
model(images)

# Train model
model.compile(
    optimizer="adam",
    loss=keras.losses.BinaryCrossentropy(from_logits=False),
    metrics=["accuracy"],
)
model.fit(images, labels, epochs=3)
```

## Example Usage with Hugging Face URI

```python
import keras_cv
import  keras
import numpy as np
```



Using the class with a `backbone`:

```python
import tensorflow as tf
import keras_cv

images = np.ones(shape=(1, 96, 96, 3))
labels = np.zeros(shape=(1, 96, 96, 1))
backbone = keras_cv.models.MiTBackbone.from_preset("hf://keras/segformer_b0_ade20k_512")
model = keras_cv.models.segmentation.SegFormer(
    num_classes=1, backbone=backbone,
)

# Evaluate model
model(images)

# Train model
model.compile(
    optimizer="adam",
    loss=keras.losses.BinaryCrossentropy(from_logits=False),
    metrics=["accuracy"],
)
model.fit(images, labels, epochs=3)
```
