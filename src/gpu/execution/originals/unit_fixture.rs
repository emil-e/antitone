//! Bounded supplied-sample geometry for reconstruction and upstream-support tests.
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct Mapping {
    pub(super) sizes: [u32; 4],
    pub(super) origin: [f32; 4],
    pub(super) dx: [f32; 4],
    pub(super) dy: [f32; 4],
    pub(super) space: [u32; 4],
}

pub(crate) struct Unit {
    pub(crate) bounds: SourceRegion,
    pub(super) mapping: Mapping,
    pub(super) support: Option<Vec<u32>>,
}

impl Unit {
    #[cfg(test)]
    pub(crate) fn new(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
    ) -> Result<Self, Error> {
        Self::planned(composition, origin, extent, &Default::default()).map(|(unit, _)| unit)
    }

    #[cfg(test)]
    pub(super) fn planned(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
        pipeline: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<(Self, Plan), Error> {
        let plan = admit_source(
            composition,
            origin,
            extent,
            pipeline,
            WorkBudget {
                source_pixels: u64::MAX,
            },
        )?;
        Ok((Self::from_plan(&plan, origin)?, plan))
    }

    fn from_plan(plan: &Plan, origin: [u32; 2]) -> Result<Self, Error> {
        let SourceRead::Originals(bounds) = plan.source_read() else {
            unreachable!("original-sample processing plan")
        };
        let composition = plan.output().composition;
        let extent = plan.output().output;
        let ValidOutput::Reconstruction {
            affine: [start, dx, dy],
        } = plan.valid_output()
        else {
            unreachable!("original-sample output reconstruction")
        };
        let frame = composition.size();
        let edge = (0..2).any(|i| origin[i] == 0 || origin[i] + extent[i] == frame[i]);
        let support = if edge {
            let coverage =
                composition.coverage(FramePoint([0.; 2]), FramePoint(frame.map(f64::from)))?;
            let count = u64::from(bounds.width) * u64::from(bounds.height);
            let len = usize::try_from(count.div_ceil(32))
                .map_err(|_| Error::InvalidParameter("processing mask exceeds address space"))?;
            let mut bits = vec![0u32; len];
            for row in 0..bounds.height {
                for col in 0..bounds.width {
                    let index = u64::from(row) * u64::from(bounds.width) + u64::from(col);
                    if coverage.weight([bounds.x + col, bounds.y + row]) > 0. {
                        bits[index as usize / 32] |= 1 << (index % 32);
                    }
                }
            }
            Some(bits)
        } else {
            None
        };
        let mapping = Mapping {
            sizes: [bounds.width, bounds.height, extent[0], extent[1]],
            origin: [start[0], start[1], 0., 0.],
            dx: [dx[0], dx[1], 0., 0.],
            dy: [dy[0], dy[1], 0., 0.],
            space: [0, u32::from(support.is_some()), 0, 0],
        };
        Ok(Self {
            bounds,
            mapping,
            support,
        })
    }
}
