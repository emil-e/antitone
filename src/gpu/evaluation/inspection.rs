//! Source viewing is an ordinary captured colour operation, not a second photographic recipe.
use super::CellRegion as Rect;
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Inspection {
    columns: [[f32; 4]; 3],
    coverage: [u32; 4],
}

pub(crate) fn inspect(
    input: Arc<ImageNode>,
    parameters: crate::gpu::ProcessingParameters,
) -> Result<Arc<ImageNode>, Error> {
    if input.domain.meaning != Meaning::Neutral {
        return Err(Error::InvalidParameter("inspection requires neutral AP1"));
    }
    let operation = Inspection {
        columns: parameters.post.map(|column| column.to_array()),
        coverage: [input.domain.grid.coverage, 0, 0, 0],
    };
    Ok(ImageNode::new(input.domain, Some(input), operation))
}

impl Operation for Inspection {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(*self))
    }

    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        Some(Function::new(include_str!("inspection.wgsl"), "inspect_source").with_uniform(0, self))
    }
}
