//! File settings, metadata and bounded encoding. No photographic graph or GPU state.
use super::{
    BlockOrder, ImageSize, RenderTarget, Result,
    samples::{FloatPrecision, Layout, Samples},
    stream,
};
use crate::{Error, profiles};
use exr::{math::Vec2, meta::attribute::Chromaticities};
use std::fs;

pub(super) fn ap0_metadata() -> (&'static str, Chromaticities) {
    (
        "scene-linear AP0",
        Chromaticities {
            red: Vec2(0.7347, 0.2653),
            green: Vec2(0., 1.),
            blue: Vec2(0.0001, -0.0770),
            white: Vec2(0.32168, 0.33767),
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Tiff,
    Exr(FloatPrecision),
}

impl FileFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Exr(_) => "exr",
        }
    }

    pub(crate) fn layout(self) -> Layout {
        match self {
            Self::Tiff => Layout::Rgb16,
            Self::Exr(precision) => Layout::Float(precision),
        }
    }

    pub(super) fn encode(
        self,
        file: &mut fs::File,
        size: ImageSize,
        target: RenderTarget,
        order: BlockOrder,
        mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
    ) -> Result<()> {
        let mismatch = || Error::InvalidParameter("samples do not match the file encoding");
        match (self, target) {
            (Self::Tiff, RenderTarget::Display(target)) => stream::tiff(
                file,
                size,
                profiles::icc(target),
                order,
                &mut |origin, extent| match block(origin, extent)? {
                    Samples::Rgb16(codes) => Ok(codes),
                    _ => Err(mismatch()),
                },
            ),
            (Self::Exr(FloatPrecision::Half), RenderTarget::SceneLinearAp0) => stream::exr(
                file,
                size,
                ap0_metadata(),
                &mut |origin, extent| match block(origin, extent)? {
                    Samples::F16(codes) => Ok(codes),
                    _ => Err(mismatch()),
                },
            ),
            (Self::Exr(FloatPrecision::Full), RenderTarget::SceneLinearAp0) => stream::exr(
                file,
                size,
                ap0_metadata(),
                &mut |origin, extent| match block(origin, extent)? {
                    Samples::F32(codes) => Ok(codes),
                    _ => Err(mismatch()),
                },
            ),
            _ => unreachable!("the export plan admits only supported target and layout pairs"),
        }
    }
}
