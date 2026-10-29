//! Captured photographic functions over neutral AP1, independent of raster acquisition.
use super::*;
use crate::gpu::{GpuGrade, ProcessingParameters};
use std::sync::{Arc, OnceLock};
use wgpu::util::DeviceExt;

pub(crate) struct Grading {
    captured: Arc<Captured>,
    active_layers: bool,
    coordinates: Option<crate::gpu::SourceGeometry>,
}

struct Captured {
    device: wgpu::Device,
    parameters: ProcessingParameters,
    layers: wgpu::Buffer,
    context: wgpu::Buffer,
    coverage: u32,
    functions: OnceLock<Functions>,
}
struct Functions {
    roll: Function,
    layers: Function,
    bytes: u64,
}

impl Captured {
    fn functions(&self) -> &Functions {
        self.functions.get_or_init(|| {
            let parameters = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("captured photographic Roll constants"),
                    contents: bytemuck::bytes_of(&self.parameters),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            // Match the filtered-view gain packing rather than changing CPU/GPU rounding.
            let values = [
                [self.parameters.anchor.w.exp().to_bits(), 0, 0, 0],
                [self.coverage, 0, 0, 0],
            ];
            let context = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("captured photographic coverage and exposure"),
                    contents: bytemuck::cast_slice(&values),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bytes = parameters.size() + context.size();
            let code: Arc<str> = Grading::shader().into();
            Functions {
                roll: Function {
                    code: code.clone(),
                    symbol: "evaluation_roll",
                    bindings: vec![
                        (2, Shared::Buffer(parameters).into()),
                        (8, Shared::Buffer(context.clone()).into()),
                    ],
                },
                layers: Function {
                    code,
                    symbol: "evaluation_adjustments",
                    bindings: vec![
                        (6, Shared::Buffer(self.layers.clone()).into()),
                        (7, Shared::Buffer(self.context.clone()).into()),
                        (8, Shared::Buffer(context).into()),
                    ],
                },
                bytes,
            }
        })
    }
}

struct Grade {
    captured: Arc<Captured>,
    roll: bool,
    coordinates: Option<crate::gpu::SourceGeometry>,
}
impl Operation for Grade {
    fn input(
        &self,
        region: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
        Ok(Some(region))
    }
    fn pointwise(&self) -> Option<Function> {
        if !self.roll && self.coordinates.is_some() {
            return None;
        }
        let functions = self.captured.functions();
        Some(
            if self.roll {
                &functions.roll
            } else {
                &functions.layers
            }
            .clone(),
        )
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let geometry = self.coordinates.ok_or(Error::InvalidParameter(
            "pointwise grading requires shader composition",
        ))?;
        let input = input.unwrap();
        let functions = self.captured.functions();
        let mut bindings = functions.layers.lower_bindings(fragment);
        bindings.extend([
            (
                5,
                fragment.uniform(&crate::gpu::flat::FieldGeometry::uncorrected(Some(
                    geometry,
                ))),
            ),
            (20, Binding::Read(input.resource)),
            (21, Binding::Write(output.resource)),
            (
                22,
                fragment.uniform(&[
                    output.region.x,
                    output.region.y,
                    output.region.width,
                    geometry.size()[0],
                ]),
            ),
        ]);
        fragment.compute(
            format!(
                "{}\n{}",
                functions.layers.code,
                include_str!("camera_adjustments.wgsl")
            ),
            "camera_adjustments",
            bindings,
            output.cells(),
        );
        Ok(())
    }
}

impl Grading {
    #[cfg(feature = "editor")]
    pub(super) fn prewarm(
        device: &wgpu::Device,
        context: &Context,
        domain: Domain,
    ) -> Result<(), Error> {
        let grade = GpuGrade::new(
            device,
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
            [0, 0, 1, 1].into(),
        )?;
        for active_layers in [false, true] {
            let grading = Self::from_parts(
                device,
                grade.parameters,
                grade.buffer.clone(),
                grade.context.clone(),
                active_layers,
                domain.grid.coverage,
            );
            context.prewarm_image(grading.compose(borrowed_input(domain))?)?;
        }
        Ok(())
    }
    pub(crate) fn shader() -> String {
        format!(
            "{}\n{}",
            crate::gpu::ProcessingKernel::kernel_shader(),
            include_str!("grading.wgsl")
        )
    }
    pub fn new(device: &wgpu::Device, grade: &GpuGrade, coverage: u32) -> Self {
        Self::from_parts(
            device,
            grade.parameters,
            grade.buffer.clone(),
            grade.context.clone(),
            grade
                .snapshot()
                .layers()
                .iter()
                .any(|layer| layer.is_active()),
            coverage,
        )
    }
    pub(super) fn from_parts(
        device: &wgpu::Device,
        parameters: ProcessingParameters,
        layers: wgpu::Buffer,
        context: wgpu::Buffer,
        active_layers: bool,
        coverage: u32,
    ) -> Self {
        Self {
            captured: Arc::new(Captured {
                device: device.clone(),
                parameters,
                layers,
                context,
                coverage,
                functions: OnceLock::new(),
            }),
            active_layers,
            coordinates: None,
        }
    }

    pub(super) fn with_camera_coordinates(
        mut self,
        geometry: Option<crate::gpu::SourceGeometry>,
    ) -> Self {
        self.coordinates = geometry;
        self
    }

    pub fn roll(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain.meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter("Roll requires neutral AP1"));
        }
        Ok(ImageNode::new(
            Domain {
                meaning: Meaning::Graded,
                ..input.domain
            },
            Some(input),
            Grade {
                captured: self.captured.clone(),
                roll: true,
                coordinates: None,
            },
        ))
    }

    pub fn layers(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain.meaning != Meaning::Graded {
            return Err(Error::InvalidParameter(
                "adjustment layers require graded AP1",
            ));
        }
        Ok(if self.active_layers {
            ImageNode::new(
                input.domain,
                Some(input),
                Grade {
                    captured: self.captured.clone(),
                    roll: false,
                    coordinates: self.coordinates,
                },
            )
        } else {
            input
        })
    }

    pub fn compose(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        self.layers(self.roll(input)?)
    }

    pub fn byte_len(&self) -> u64 {
        self.captured
            .functions
            .get()
            .map_or(0, |functions| functions.bytes)
    }
}
