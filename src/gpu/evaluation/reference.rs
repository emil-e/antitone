//! Captured creative look and output-reference composition, independent of photographic grading.
use super::*;
use crate::rendering::{OutputReference, OutputTarget};

pub(crate) struct Reference {
    target: Option<OutputTarget>,
    look: Option<Arc<crate::gpu::look::Lut>>,
}
impl Reference {
    pub fn prewarm(context: &Context) -> Result<(), Error> {
        context.prewarm(&[Look::work(
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
        asset: Option<&Arc<crate::looks::LookAsset>>,
    ) -> Result<Self, Error> {
        let target = reference.display_target();
        let look = asset
            .filter(|_| target.is_some())
            .map(|asset| renderer.look_resource(device, asset))
            .transpose()?;
        Ok(Self { target, look })
    }
    pub fn look(&self) -> Option<Arc<crate::gpu::look::Lut>> {
        self.look.clone()
    }
    pub fn compose(
        &self,
        image: Arc<ImageNode>,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<Arc<ImageNode>, Error> {
        let Some(target) = self.target else {
            return Ok(image);
        };
        display(self.looked(image, None), renderer, target)
    }
    /// Caller supplies fresh admission status in slot one for each encoding, not per recipe.
    pub fn presentation(
        &self,
        image: Arc<ImageNode>,
        renderer: &crate::renderer::GpuRenderer,
        texture: &wgpu::Texture,
    ) -> Result<Arc<ImageNode>, Error> {
        let target = self.target.ok_or(Error::InvalidParameter(
            "presentation requires a display reference",
        ))?;
        operators::display_texture(self.looked(image, Some(1)), renderer, target, texture)
    }
    fn looked(&self, image: Arc<ImageNode>, status_slot: Option<usize>) -> Arc<ImageNode> {
        match &self.look {
            Some(look) => ImageNode::new(
                image.domain,
                Some(image.clone()),
                Look {
                    look: look.clone(),
                    status_slot,
                },
            ),
            None => image,
        }
    }
}
struct Look {
    look: Arc<crate::gpu::look::Lut>,
    status_slot: Option<usize>,
}
impl Look {
    fn work(
        input: Resource,
        output: Resource,
        lut: Binding,
        parameters: Binding,
        status: Resource,
        count: u32,
    ) -> Work {
        Work::linear(
            include_str!("../../shaders/look.wgsl"),
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
impl Operation for Look {
    fn input(
        &self,
        r: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
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
        let status = f.resource(match self.status_slot {
            Some(slot) => Storage::Input {
                slot,
                bytes: 8,
                writable: true,
            },
            None => Storage::Data(bytemuck::cast_slice(&[0u32, u32::MAX]).into()),
        });
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
