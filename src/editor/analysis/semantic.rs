//! Segment the display proxy to admit original-site white-balance evidence.
use crate::Error;
use ort::{session::Session, value::Tensor};
use std::path::{Path, PathBuf};

const MODEL: &[u8] = include_bytes!("../../../assets/analysis/segformer-b0.onnx");
const INPUT: usize = 512;
const OUTPUT: usize = 128;
const CLASSES: usize = 150;

pub(super) struct Classifier(Session);

fn runtime_error(error: impl std::fmt::Display) -> Error {
    Error::Rendering(format!("Analyze semantic WB: {error}"))
}

fn runtime_library() -> Result<PathBuf, Error> {
    let executable = std::env::current_exe().map_err(runtime_error)?;
    let directory = executable
        .parent()
        .ok_or_else(|| runtime_error("cannot locate the application directory"))?;
    let (packaged, filename) = if cfg!(target_os = "macos") {
        (directory.join("../Frameworks"), "libonnxruntime.dylib")
    } else {
        (directory.to_owned(), "onnxruntime.dll")
    };

    let profile = Path::new(env!("OUT_DIR")).ancestors().nth(3);
    let in_cargo_output = profile.is_some_and(|p| {
        directory == p
            || (directory.file_name().is_some_and(|n| n == "deps") && directory.parent() == Some(p))
    });
    let library = if in_cargo_output {
        let version = include_str!("../../../native/onnxruntime/pins.txt")
            .lines()
            .find_map(|line| line.strip_prefix("version="))
            .expect("ONNX Runtime version pin");
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".cache/native-deps/onnxruntime-{version}"))
            .join(filename)
    } else {
        packaged.join(filename)
    };
    library
        .canonicalize()
        .map_err(|error| runtime_error(format!("{}: {error}", library.display())))
}

/// Package verification exercises the same CPU session without opening a window.
pub fn check_runtime() -> Result<PathBuf, Error> {
    let library = runtime_library()?;
    let mut classifier = Classifier::open(&library)?;
    let preview = image::RgbImage::from_pixel(16, 16, image::Rgb([128; 3]));
    classifier.keep(&preview, &[[0.5, 0.5]], &mut || false)?;
    Ok(library)
}

impl Classifier {
    pub(super) fn load() -> Result<Self, Error> {
        Self::open(&runtime_library()?)
    }

    fn open(library: &Path) -> Result<Self, Error> {
        ort::init_from(library).map_err(runtime_error)?.commit();
        let session = Session::builder()
            .map_err(runtime_error)?
            .with_intra_threads(4)
            .map_err(runtime_error)?
            .with_inter_threads(1)
            .map_err(runtime_error)?
            .commit_from_memory(MODEL)
            .map_err(runtime_error)?;
        Ok(Self(session))
    }

    pub(super) fn keep(
        &mut self,
        preview: &image::RgbImage,
        sites: &[[f64; 2]],
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec<bool>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let pixels = image::imageops::resize(
            preview,
            INPUT as u32,
            INPUT as u32,
            image::imageops::FilterType::Triangle,
        );
        let mut data = vec![0.; 3 * INPUT * INPUT];
        let mean = [123.675, 116.28, 103.53];
        let sd = [58.395, 57.12, 57.375];
        for (i, pixel) in pixels.pixels().enumerate() {
            for c in 0..3 {
                data[c * INPUT * INPUT + i] = (f32::from(pixel[c]) - mean[c]) / sd[c];
            }
        }
        let input = Tensor::from_array(([1, 3, INPUT, INPUT], data)).map_err(runtime_error)?;
        let output = self
            .0
            .run(ort::inputs!["pixels" => input])
            .map_err(runtime_error)?;
        if cancelled() {
            return Ok(None);
        }
        if output.len() != 1 {
            return Err(runtime_error("expected one logits output"));
        }
        let (shape, logits) = output[0]
            .try_extract_tensor::<f32>()
            .map_err(runtime_error)?;
        if &shape[..] != [1, CLASSES as i64, OUTPUT as i64, OUTPUT as i64] {
            return Err(runtime_error("unexpected logit raster"));
        }
        sites
            .iter()
            .map(|&site| keep_site(logits, site))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn keep_site(logits: &[f32], point: [f64; 2]) -> Result<bool, Error> {
    let (class, confidence, vegetation) = site_class(logits, point)?;
    Ok(keep_class(class, confidence, vegetation))
}

fn keep_class(class: usize, confidence: f32, vegetation: f32) -> bool {
    vegetation < 0.75 && (!matches!(class, 2 | 13) || confidence < 0.75)
}

fn site_class(logits: &[f32], point: [f64; 2]) -> Result<(usize, f32, f32), Error> {
    let [x, y] = point.map(|v| (v * OUTPUT as f64 - 0.5).clamp(0., (OUTPUT - 1) as f64));
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(OUTPUT - 1), (y0 + 1).min(OUTPUT - 1));
    let (fx, fy) = (x.fract() as f32, y.fract() as f32);
    let mut values = [0.; CLASSES];
    let (mut top, mut peak) = (0, f32::NEG_INFINITY);
    for (i, plane) in logits.chunks_exact(OUTPUT * OUTPUT).enumerate() {
        let a = (1. - fx) * plane[y0 * OUTPUT + x0] + fx * plane[y0 * OUTPUT + x1];
        let b = (1. - fx) * plane[y1 * OUTPUT + x0] + fx * plane[y1 * OUTPUT + x1];
        let v = (1. - fy) * a + fy * b;
        if !v.is_finite() {
            return Err(runtime_error("nonfinite logits"));
        }
        values[i] = v;
        if v > peak {
            top = i;
            peak = v;
        }
    }
    let weights = values.map(|v| (v - peak).exp());
    let total: f32 = weights.iter().sum();
    // ADE20K tree, grass, plant, flower and palm.
    let vegetation: f32 = [4, 9, 17, 66, 72].into_iter().map(|i| weights[i]).sum();
    Ok((top, total.recip(), vegetation / total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_vegetation_confidence_can_veto_a_site() {
        let mut scores = vec![0.; CLASSES * OUTPUT * OUTPUT];
        scores[4 * OUTPUT * OUTPUT] = 8.;
        scores[17 * OUTPUT * OUTPUT] = 8.;
        let (_, top, vegetation) = site_class(&scores, [0., 0.]).unwrap();
        assert!(top < 0.75 && vegetation >= 0.75);
        assert!(!keep_site(&scores, [0., 0.]).unwrap());
    }

    #[test]
    fn filters_only_a_confident_class_at_the_requested_site() {
        let mut scores = vec![0.; CLASSES * OUTPUT * OUTPUT];
        scores[13 * OUTPUT * OUTPUT] = 12.;
        assert!(!keep_site(&scores, [0., 0.]).unwrap());
        assert!(keep_site(&scores, [1., 1.]).unwrap());
        assert!(keep_site(&scores, [1. / OUTPUT as f64, 1. / OUTPUT as f64]).unwrap());
        scores[21 * OUTPUT * OUTPUT] = 16.;
        assert!(keep_site(&scores, [0., 0.]).unwrap());
    }
}
