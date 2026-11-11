//! Captured creative look and output-reference composition, independent of photographic grading.

use super::*;
use crate::rendering::{OutputReference, OutputTarget};

pub struct Reference {
    target: Option<OutputTarget>,
    id: Option<crate::looks::LookId>,
    luts: Vec<Arc<crate::gpu::lut3d::Lut>>,
}

impl Reference {
    #[cfg(all(test, target_os = "macos", feature = "raw"))]
    pub(crate) fn image_lut_bytes(image: &Value) -> u64 {
        let mut bytes = 0;
        let mut image = Some(image);
        while let Some(node) = image {
            if let Some(lut) =
                (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<LutOperation>()
            {
                bytes += lut.look.byte_len();
            }
            image = node.unary_input();
        }
        bytes
    }

    pub fn prewarm(
        context: &Context,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<(), Error> {
        crate::gpu::lut3d::prewarm(context)?;
        let domain = Domain {
            bounds: Some(CellRegion::new([0, 0], [1, 1])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        };
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            for looked in [false, true] {
                let input = borrowed_input(domain);
                let input = if looked {
                    value::pointwise(
                        input,
                        Meaning::Graded,
                        crate::gpu::lut3d::prewarm_calculation(false),
                    )?
                } else {
                    input
                };
                context.prewarm_texture(display(input, renderer, target)?)?;
            }
        }
        Ok(())
    }

    pub fn capture(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        reference: OutputReference,
        assets: &crate::looks::LutChain,
    ) -> Result<Self, Error> {
        let target = reference.display_target();
        let luts = if target.is_some() {
            assets
                .assets()
                .map(|asset| renderer.look_resource(device, asset))
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        Ok(Self {
            target,
            id: target.and_then(|_| assets.id()),
            luts,
        })
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.id
    }

    pub fn lut_resources(&self) -> Vec<Arc<crate::gpu::lut3d::Lut>> {
        self.luts.clone()
    }

    pub fn compose(
        &self,
        image: Value,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<Value, Error> {
        let Some(target) = self.target else {
            return Ok(image);
        };
        display(self.looked(image), renderer, target)
    }

    pub fn looked(&self, mut image: Value) -> Value {
        for lut in &self.luts {
            image = Value::from_operation(
                image.domain,
                Some(image.clone()),
                LutOperation { look: lut.clone() },
            );
        }
        image
    }
}

struct LutOperation {
    look: Arc<crate::gpu::lut3d::Lut>,
}

impl Operation for LutOperation {
    fn key(&self) -> Option<RetainedKey> {
        {
            #[derive(Debug, PartialEq)]
            struct Content(crate::looks::LookId);
            Some(RetainedKey::new(Content(self.look.id())))
        }
    }

    fn reads(&self, r: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, r)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            self.look.calculation(domain.grid.coverage != 0),
        )
        .map(Some)
    }
}
