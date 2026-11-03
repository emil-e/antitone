//! Captured photographic functions over neutral AP1, independent of raster acquisition.

use crate::Error;
use crate::gpu::evaluation::*;
use crate::gpu::grading::Grading;
use std::sync::Arc;

struct Functions {
    roll: Function,
    layers: Function,
}

impl Grading {
    fn functions(&self) -> Functions {
        // Match the filtered-view gain packing rather than changing CPU/GPU rounding.
        let context = [
            [self.parameters.anchor.w.exp().to_bits(), 0, 0, 0],
            [self.coverage, 0, 0, 0],
        ];
        let code: Arc<str> = Grading::shader().into();
        Functions {
            roll: Function::new(code.clone(), "evaluation_roll")
                .with_uniform(2, &self.parameters)
                .with_uniform(8, &context),
            layers: Function::new(code, "evaluation_adjustments")
                .with_storage(6, self.layers.clone())
                .with_uniform(7, &self.context)
                .with_uniform(8, &context),
        }
    }
}

struct Grade {
    definition: GradeDefinition,
    function: Function,
}

#[derive(Clone, Debug, PartialEq)]
enum GradeDefinition {
    Roll(crate::processing::RollSettings, u32),
    Layers {
        layers: crate::adjustment_layers::AdjustmentLayerSnapshot,
        area: crate::geometry::SourceRegion,
        coverage: u32,
        coordinates: Option<crate::gpu::source::SourceGeometry>,
    },
}

impl Grade {
    fn coordinates(&self) -> Option<crate::gpu::source::SourceGeometry> {
        match self.definition {
            GradeDefinition::Layers { coordinates, .. } => coordinates,
            GradeDefinition::Roll(..) => None,
        }
    }
}

impl Operation for Grade {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.definition.clone()))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        self.coordinates().is_none().then(|| self.function.clone())
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let geometry = self.coordinates().ok_or(Error::InvalidParameter(
            "pointwise grading requires shader composition",
        ))?;
        let input = input.unwrap();
        let mut bindings = self.function.lower_bindings(fragment);
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
                    output.region.x as u32,
                    output.region.y as u32,
                    output.region.width,
                    geometry.size()[0],
                ]),
            ),
        ]);
        fragment.compute(
            format!(
                "{}\n{}",
                self.function.code,
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
    pub(crate) fn prewarm(
        device: &wgpu::Device,
        context: &Context,
        domain: Domain,
    ) -> Result<(), Error> {
        let grade = Self::new(
            &device.limits(),
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
            [0, 0, 1, 1].into(),
        )?;
        for active_layers in [false, true] {
            let mut grading = grade.for_grid(domain.grid.coverage);
            grading.active_layers = active_layers;
            context.prewarm_image(grading.compose(borrowed_input(domain))?)?;
        }
        Ok(())
    }

    pub(crate) fn shader() -> String {
        format!(
            "{}\n{}",
            crate::gpu::photograph::shader::kernel_shader(),
            include_str!("grading.wgsl")
        )
    }

    pub fn for_grid(&self, coverage: u32) -> Self {
        let mut prepared = self.clone();
        prepared.coverage = coverage;
        prepared
    }

    pub(crate) fn with_camera_coordinates(
        mut self,
        geometry: Option<crate::gpu::source::SourceGeometry>,
    ) -> Self {
        self.coordinates = geometry;
        self
    }

    pub fn roll(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain().meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter("Roll requires neutral AP1"));
        }
        Ok(ImageNode::new(
            Domain {
                meaning: Meaning::Graded,
                ..input.domain()
            },
            Some(input),
            Grade {
                definition: GradeDefinition::Roll(self.roll, self.coverage),
                function: self.functions().roll.clone(),
            },
        ))
    }

    pub fn layers(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain().meaning != Meaning::Graded {
            return Err(Error::InvalidParameter(
                "adjustment layers require graded AP1",
            ));
        }
        Ok(if self.active_layers {
            ImageNode::new(
                input.domain(),
                Some(input),
                Grade {
                    definition: GradeDefinition::Layers {
                        layers: self.snapshot.clone(),
                        area: self.area,
                        coverage: self.coverage,
                        coordinates: self.coordinates,
                    },
                    function: self.functions().layers.clone(),
                },
            )
        } else {
            input
        })
    }

    pub fn compose(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        self.layers(self.roll(input)?)
    }
}
