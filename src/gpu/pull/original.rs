//! Original-camera operators. Existing photographic, repair, grain and look shaders own the math.
use super::*;
use crate::{
    geometry::SourceRegion as Rect,
    gpu::{GpuFlatField, ProcessingInput, SourceGeometry, flat::FieldGeometry},
};

struct Native {
    scan: crate::scan::Scan,
}
impl Source for Native {
    fn domain(&self) -> Domain {
        Domain {
            bounds: self.scan.area,
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let values = self.scan.tile_region(
            self.scan.area,
            region.x - self.scan.area.x,
            region.y - self.scan.area.y,
            region.width,
            region.height,
        )?;
        crate::gpu::ProcessingSource::validate_continuous(&values)?;
        Ok(values)
    }
}

pub(crate) fn original_chain(
    device: &wgpu::Device,
    renderer: &crate::renderer::GpuRenderer,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    input: &ProcessingInput,
    reference: crate::rendering::OutputReference,
    stages: &crate::pixel_pipeline::PixelPipeline,
) -> Result<Arc<Layer>, Error> {
    if scan.flat_field() != field.as_ref().map(|f| f.field()) {
        return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
    }
    let matrix = if let Some(pipeline) = input.pipeline() {
        if !pipeline.input_program().matches_native(scan)? {
            return Err(Error::Gpu(
                "original input differs from captured camera source".into(),
            ));
        }
        pipeline.grade.require_extent(scan.area)?;
        pipeline.validate_repairs(device, input.repairs.bound())?;
        pipeline.input_program().matrix
    } else {
        scan.camera_to_ap1
    };
    input.repairs.require_area(scan.area)?;
    crate::gpu::CameraMatrix::new(matrix * crate::processing::MAX_SOURCE_GAIN)?;
    if let Some(field) = &field {
        field.validate_matrix(matrix, crate::processing::MAX_SOURCE_GAIN)?;
    }
    let mut snapshot =
        crate::scan::Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
    snapshot.set_flat_field(scan.flat_field().cloned())?;
    let mut node = source(Arc::new(Native { scan: snapshot }));
    let empty = || {
        (
            renderer.kernel.empty_adjustment_layers.clone(),
            renderer.kernel.empty_adjustment_layer_context.clone(),
        )
    };
    let (layers, context) = input
        .grade()
        .map(|g| (g.buffer.clone(), g.context.clone()))
        .unwrap_or_else(empty);
    let mut camera = crate::gpu::CameraMatrix::new(matrix)?;
    camera.columns[0].w = 1.;
    let picture = Photographic {
        input: input.clone(),
        matrix: camera,
        field,
        nodes: crate::gpu::flat::identity_nodes(device),
        layers,
        context,
        source_limit: device
            .limits()
            .max_storage_buffer_binding_size
            .min(device.limits().max_buffer_size)
            / 16,
        code: crate::gpu::ProcessingKernel::module_shader().into(),
    };
    node = Layer::new(
        Domain {
            meaning: if input.parameters().graded() {
                Meaning::Graded
            } else {
                Meaning::Neutral
            },
            ..node.domain
        },
        Some(node),
        picture,
    );
    node = stages.compose(node)?;
    if let Some(target) = reference.display_target() {
        if let Some(asset) = input.look() {
            let look = renderer.look_resource(device, asset)?;
            node = Layer::new(node.domain, Some(node.clone()), Look { look });
        }
        node = display(node, renderer, target)?;
    }
    Ok(node)
}

struct Photographic {
    input: ProcessingInput,
    matrix: crate::gpu::CameraMatrix,
    field: Option<Arc<GpuFlatField>>,
    nodes: wgpu::Buffer,
    layers: wgpu::Buffer,
    context: wgpu::Buffer,
    source_limit: u64,
    code: Arc<str>,
}
impl Operation for Photographic {
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }
    fn lower(&self, f: &mut Fragment, input: Option<Image>, output: Image) -> Result<(), Error> {
        let input = input.unwrap();
        if u64::from(input.cells()) > self.source_limit {
            return Err(Error::InvalidParameter(
                "processing unit exceeds source work budget",
            ));
        }
        let r = input.region;
        let geometry = SourceGeometry::original_tile([r.x, r.y, r.width, r.height], input.size())?;
        let field = match &self.field {
            Some(field) => FieldGeometry::new(field, geometry),
            None => FieldGeometry::uncorrected(Some(geometry)),
        };
        let parameters = f.uniform(&self.input.parameters());
        let matrix = f.uniform(&self.matrix);
        let coordinates = f.uniform(&field);
        f.compute(
            self.code.clone(),
            "main",
            vec![
                (0, Binding::Read(input.resource)),
                (1, Binding::Write(output.resource)),
                (2, parameters),
                (3, matrix),
                (
                    4,
                    Binding::Shared(Shared::Buffer(
                        self.field
                            .as_ref()
                            .map_or(&self.nodes, |v| &v.nodes)
                            .clone(),
                    )),
                ),
                (5, coordinates),
                (6, Binding::Shared(Shared::Buffer(self.layers.clone()))),
                (7, Binding::Shared(Shared::Buffer(self.context.clone()))),
            ],
            output.cells(),
        );
        if !self.input.parameters().graded() || self.input.repairs.0.is_empty() {
            return Ok(());
        }
        let grade = self.input.grade().ok_or(Error::InvalidParameter(
            "repair grading requires a photographic grade",
        ))?;
        let parameters = f.uniform(&grade.parameters);
        let geometry = f.uniform(&FieldGeometry::uncorrected(Some(geometry)));
        let tile = f.uniform(&[r.x, r.y, r.width, r.height, 0u32, 0, 0, 0]);
        for patch in &self.input.repairs.0 {
            let pixels = patch.fragment_pixels(r);
            if pixels.is_empty() {
                continue;
            }
            let count = pixels.len() as u32;
            let data = f.resource(Storage::Data(bytemuck::cast_slice(&pixels).into()));
            f.compute(
                self.code.clone(),
                "repair_original",
                vec![
                    (1, Binding::Write(output.resource)),
                    (2, parameters.clone()),
                    (5, geometry.clone()),
                    (6, Binding::Shared(Shared::Buffer(grade.buffer.clone()))),
                    (7, Binding::Shared(Shared::Buffer(grade.context.clone()))),
                    (12, Binding::Read(data)),
                    (13, tile.clone()),
                ],
                count,
            );
        }
        Ok(())
    }
}

struct Look {
    look: Arc<crate::gpu::look::Lut>,
}
impl Operation for Look {
    fn input(&self, r: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(r))
    }
    fn lower(&self, f: &mut Fragment, input: Option<Image>, output: Image) -> Result<(), Error> {
        let input = input.unwrap();
        let parameters = f.uniform(&self.look.parameters(
            output.cells(),
            output.cells().div_ceil(64) * 64,
            false,
        ));
        let status = f.resource(Storage::Data(
            bytemuck::cast_slice(&[0u32, u32::MAX]).into(),
        ));
        f.compute(
            include_str!("../../shaders/look.wgsl"),
            "main",
            vec![
                (0, Binding::Read(input.resource)),
                (1, Binding::Write(output.resource)),
                (2, Binding::Shared(Shared::Buffer(self.look.buffer.clone()))),
                (3, parameters),
                (4, Binding::Write(status)),
            ],
            output.cells(),
        );
        Ok(())
    }
}
