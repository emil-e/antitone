# GPU Bayer DHT

[`dht.wgsl`](../../src/gpu/demosaic/dht.wgsl) derives from Anton Petrusevich's Bayer DHT, copyright 2013, in [LibRaw 0.22.3](https://github.com/LibRaw/LibRaw/blob/cf02d66eb5c19d4e46df7aee0582e47a4918509a/src/demosaic/dht_demosaic.cpp). Antitone selects the upstream LGPL 2.1 option; [LICENSE.LGPL-2.1](LICENSE.LGPL-2.1) retains its terms. The top-level MIT license does not relicense this shader.

The port retains the ratio-based reconstruction and [corrected ranges and borders](../libraw/README.md). Three immutable hot-sample passes handle adjacent defects; final horizontal/vertical and diagonal refinements read snapshots, while the two checkerboard refinements retain their intervening barrier. These are parallel implementation choices, qualified against weak-channel texture, independent colour detail and analytic defects, rather than exact preservation of LibRaw's scan-order feedback. Measured sites come from the original prepared image. Fourth green retains the pre-border value; RGB borders average measured neighbours.

Ordinary f32 arithmetic can reverse near-tied direction comparisons and produce sparse larger differences from CPU DHT. Numerical identity is not the photographic acceptance criterion. [`raw_demosaic.rs`](../../tests/raw_demosaic.rs) checks weak-channel noise, luminance modulation, independent blue detail, chromatic edges and composed export. GPU feature builds exercise the accelerated source; native-only builds exercise the CPU fallback.

The [Rust owner](../../src/gpu/demosaic.rs) receives the application's device, guarded queue and working allowance. LibRaw supplies prepared RGBG16 through a synchronous callback and resumes CPU development after readback. [Native development](../../docs/native-development.md) describes admission, lifetime and scheduling.

Packages retain this provenance, the shader and license. Before public binary distribution, review corresponding-source and relinking obligations alongside LibRaw and RawSpeed; copying license notices alone does not satisfy them.
