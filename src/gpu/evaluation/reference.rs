//! Captured creative look and output-reference composition, independent of photographic grading.

use super::*;
use crate::rendering::{OutputReference, OutputTarget};

pub(crate) struct Reference {
    target: Option<OutputTarget>,
    id: Option<crate::looks::LookId>,
    luts: Vec<Arc<crate::gpu::lut3d::Lut>>,
}

impl Reference {
    #[cfg(all(test, target_os = "macos", feature = "raw"))]
    pub(crate) fn image_lut_bytes(image: &Arc<ImageNode>) -> u64 {
        let mut bytes = 0;
        let mut image = Some(image);
        while let Some(node) = image {
            if let Some(lut) =
                (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<LutOperation>()
            {
                bytes += lut.look.byte_len();
            }
            image = node.input();
        }
        bytes
    }

    pub fn prewarm(context: &Context) -> Result<(), Error> {
        context.prewarm(&[LutOperation::work(
            Resource(0),
            Resource(1),
            Binding::Read(Resource(2)),
            Binding::Uniform(Resource(3)),
            Resource(4),
            1,
        )])
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
        image: Arc<ImageNode>,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<Arc<ImageNode>, Error> {
        let Some(target) = self.target else {
            return Ok(image);
        };
        display(self.looked(image), renderer, target)
    }

    pub fn presentation(
        &self,
        image: Arc<ImageNode>,
        renderer: &crate::renderer::GpuRenderer,
        texture: &wgpu::Texture,
        region: CellRegion,
    ) -> Result<Arc<ImageNode>, Error> {
        let target = self.target.ok_or(Error::InvalidParameter(
            "presentation requires a display reference",
        ))?;
        operators::display_texture(self.looked(image), renderer, target, texture, region)
    }

    pub fn looked(&self, mut image: Arc<ImageNode>) -> Arc<ImageNode> {
        for lut in &self.luts {
            image = ImageNode::new(
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

impl LutOperation {
    fn work(
        input: Resource,
        output: Resource,
        lut: Binding,
        parameters: Binding,
        status: Resource,
        count: u32,
    ) -> Work {
        Work::linear(
            include_str!("../lut3d.wgsl"),
            "main",
            vec![
                (0, Binding::Read(input)),
                (1, Binding::Write(output)),
                (2, lut),
                (3, parameters),
                (4, Binding::Write(status)),
            ],
            count,
        )
    }
}

impl Operation for LutOperation {
    fn identity(&self) -> Option<RetainedKey> {
        #[derive(Debug, PartialEq)]
        struct Content(crate::looks::LookId);
        Some(RetainedKey::new(Content(self.look.id())))
    }

    fn input(&self, r: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(r))
    }

    fn lower(
        &self,
        f: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let parameters = f.uniform(&self.look.parameters(
            output.cells(),
            output.cells().div_ceil(64) * 64,
            input.domain.grid.coverage != 0,
        ));
        let status = f.resource(Storage::Data(
            bytemuck::cast_slice(&[0u32, u32::MAX]).into(),
        ));
        f.work.push(Self::work(
            input.resource,
            output.resource,
            Binding::Shared(Shared::Buffer(self.look.buffer.clone())),
            parameters,
            status,
            output.cells(),
        ));
        Ok(())
    }
}
