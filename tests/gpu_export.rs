#![cfg(all(
    feature = "raw",
    feature = "gpu",
    feature = "export",
    target_os = "macos"
))]
use antitone::{
    evaluation::{self, CellRegion, Domain, Grid, Source},
    export::{FileFormat, FloatPrecision, JpegQuality, Overwrite, Plan, RenderTarget},
    geometry::Composition,
    gpu_export::Request,
    rendering::OutputTarget,
};
use glam::DVec3;
use std::{path::Path, sync::Arc};
use tiff::decoder::{Decoder, DecodingResult};

struct Linear {
    pixels: Vec<[f32; 4]>,
}

impl Source for Linear {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [self.pixels.len() as u32, 1])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, antitone::Error> {
        assert_eq!(region.y, 0);
        assert_eq!(region.height, 1);
        let start = usize::try_from(region.x).unwrap();
        Ok(self.pixels[start..start + region.width as usize].to_vec())
    }
}

fn context() -> (evaluation::Context, antitone::gpu::submission::Queue) {
    pollster::block_on(async {
        let adapter = antitone::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) =
            antitone::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
        (evaluation::Context::new(&device, Default::default()), queue)
    })
}

fn composition(width: u32) -> Composition {
    Composition::new(
        [0, 0, width, 1].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap()
}

fn tiff(path: &Path) -> Vec<u16> {
    let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let DecodingResult::U16(values) = decoder.read_image().unwrap() else {
        panic!("not RGB16")
    };
    values
}

#[test]
fn file_writer_executes_already_authored_linear_nodes_without_photographic_reinterpretation() {
    let (context, queue) = context();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authored.tif");
    let request = |gain: f32| Request {
        definition: evaluation::source(Arc::new(Linear {
            pixels: vec![[0.001 * gain, 0.002 * gain, 0.003 * gain, 0.]; 2],
        })),
        composition: composition(2),
        plan: Plan::new(RenderTarget::Display(OutputTarget::Srgb), FileFormat::Tiff).unwrap(),
    };
    // Below the sRGB knee, transfer is exactly 12.92*x, followed by nearest RGB16.
    for (gain, expected) in [(1., [847, 1693, 2540]), (0.5, [423, 847, 1270])] {
        antitone::gpu_export::write(&context, &queue, request(gain), &path, Overwrite::Confirmed)
            .unwrap();
        assert_eq!(tiff(&path), expected.repeat(2));
    }
    let published = std::fs::read(&path).unwrap();
    let stopped = antitone::gpu_export::write_checked(
        &context,
        &queue,
        request(1.),
        &path,
        Overwrite::Confirmed,
        || Err(antitone::Error::Cancelled),
    );
    assert!(matches!(stopped, Err(antitone::Error::Cancelled)));
    assert_eq!(std::fs::read(path).unwrap(), published);
}

#[test]
fn jpeg_uses_the_same_authored_display_value_as_tiff() {
    use image::{ImageDecoder, codecs::jpeg::JpegDecoder};
    use std::{fs::File, io::BufReader};

    let (context, queue) = context();
    let directory = tempfile::tempdir().unwrap();
    let width = 137;
    let definition = evaluation::source(Arc::new(Linear {
        pixels: (0..width)
            .map(|x| {
                let linear = if x < 128 { 0.18 } else { 0.5 };
                [linear, linear, linear, 0.]
            })
            .collect(),
    }));
    let target = OutputTarget::DisplayP3;
    let write = |format: FileFormat| {
        let path = directory
            .path()
            .join("authored")
            .with_extension(format.extension());
        antitone::gpu_export::write(
            &context,
            &queue,
            Request {
                definition: definition.clone(),
                composition: composition(width),
                plan: Plan::new(RenderTarget::Display(target), format).unwrap(),
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        path
    };

    let jpeg = write(FileFormat::Jpeg(JpegQuality::new(100).unwrap()));
    let mut decoder = JpegDecoder::new(BufReader::new(File::open(jpeg).unwrap())).unwrap();
    assert_eq!(decoder.dimensions(), (width, 1));
    assert_eq!(
        decoder.icc_profile().unwrap().as_deref(),
        Some(antitone::profiles::icc(target))
    );
    let mut pixels = vec![0; decoder.total_bytes() as usize];
    decoder.read_image(&mut pixels).unwrap();
    for (x, expected) in [(10, 118), (136, 188)] {
        assert!(
            pixels[x * 3..x * 3 + 3]
                .iter()
                .all(|v| v.abs_diff(expected) <= 2)
        );
    }

    let pixels = tiff(&write(FileFormat::Tiff));
    assert_eq!(&pixels[30..33], &[30235; 3]);
    assert_eq!(&pixels[408..411], &[48192; 3]);
}

#[test]
fn gpu_tiff_preserves_codes_across_full_blocks_and_partial_edges() {
    struct Ramp;
    const SIZE: [u32; 2] = [519, 527];

    fn code(x: u32, y: u32) -> [u16; 3] {
        [(x * 97) as u16, (y * 101) as u16, (x + 2 * y) as u16]
    }

    impl Source for Ramp {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some(CellRegion::new([0, 0], SIZE)),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            }
        }

        fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, antitone::Error> {
            Ok((region.y..region.bottom())
                .flat_map(|y| (region.x..region.right()).map(move |x| code(x as u32, y as u32)))
                .map(|rgb| {
                    let [r, g, b] = rgb
                        .map(|c| OutputTarget::Srgb.decode(f64::from(c) / 65535.).unwrap() as f32);
                    [r, g, b, 0.]
                })
                .collect())
        }
    }

    let (context, queue) = context();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ramp.tif");
    let [width, height] = SIZE;
    antitone::gpu_export::write(
        &context,
        &queue,
        Request {
            definition: evaluation::source(Arc::new(Ramp)),
            composition: Composition::new(
                [0, 0, width, height].into(),
                Default::default(),
                Default::default(),
            )
            .unwrap(),
            plan: Plan::new(RenderTarget::Display(OutputTarget::Srgb), FileFormat::Tiff).unwrap(),
        },
        &path,
        Overwrite::Refuse,
    )
    .unwrap();

    let expected: Vec<_> = (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| code(x, y)))
        .collect();
    assert_eq!(tiff(&path), expected);
}

#[test]
fn one_authored_value_exports_to_both_scene_linear_sample_layouts() {
    use exr::prelude::{FlatSamples, Vec2, read_first_flat_layer_from_file};

    let (context, queue) = context();
    let directory = tempfile::tempdir().unwrap();
    // Basis colours expose a missing/doubled matrix or channel permutation;
    // signed/HDR and non-half-exact values expose clipping or premature packing.
    let pixels = vec![
        [0., 0., 0., 0.],
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [-0.125, -0.125, -0.125, 0.],
        [4., 4., 4., 0.],
        [0.5001, 0.10003, -0.2501, 0.],
        [70000., 70000., 70000., 0.],
        [-70000., -70000., -70000., 0.],
    ];
    let definition = evaluation::source(Arc::new(Linear {
        pixels: pixels.clone(),
    }));
    for precision in [FloatPrecision::Half, FloatPrecision::Full] {
        let path = directory.path().join(format!("{precision:?}.exr"));
        antitone::gpu_export::write(
            &context,
            &queue,
            Request {
                definition: definition.clone(),
                composition: composition(pixels.len() as u32),
                plan: Plan::new(RenderTarget::SceneLinearAp0, FileFormat::Exr(precision)).unwrap(),
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(&path).unwrap();
        assert_eq!(image.layer_data.size, Vec2(pixels.len(), 1));
        let chroma = image.attributes.chromaticities.unwrap();
        assert_eq!(chroma.white, Vec2(0.32168, 0.33767));
        assert_eq!(chroma.red, Vec2(0.7347, 0.2653));
        assert_eq!(chroma.green, Vec2(0., 1.));
        assert_eq!(chroma.blue, Vec2(0.0001, -0.077));
        assert_eq!(image.layer_data.channel_data.list.len(), 3);
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                name => panic!("unexpected EXR channel {name}"),
            };
            for (i, pixel) in pixels.iter().enumerate() {
                let supplied = DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                let expected = (antitone::color::ap1_to_ap0() * supplied)[c];
                match (&channel.sample_data, precision) {
                    (FlatSamples::F16(samples), FloatPrecision::Half) => {
                        let bounded = expected.clamp(-65504., 65504.);
                        assert_eq!(samples[i], half::f16::from_f64(bounded));
                    }
                    (FlatSamples::F32(samples), FloatPrecision::Full) => {
                        assert_eq!(samples[i], expected as f32);
                    }
                    _ => panic!("wrong EXR channel storage precision"),
                }
            }
        }
    }
}
