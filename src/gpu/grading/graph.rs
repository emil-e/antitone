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
    /// Conservative charge for grading arguments retained by a calculation, including shared parents.
    #[cfg(feature = "editor")]
    pub(crate) fn captured_bytes(image: &Value) -> u64 {
        let mut bytes = 0;
        for node in image.ancestors() {
            if let Some(grade) =
                (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<Grade>()
            {
                bytes += grade
                    .function
                    .bindings
                    .iter()
                    .map(|(_, binding)| match binding {
                        FunctionBinding::Uniform(data) | FunctionBinding::Storage(data) => {
                            data.len() as u64
                        }
                        FunctionBinding::Shared(_) => 0,
                    })
                    .sum::<u64>();
                if let GradeDefinition::Layers { layers, .. } = &grade.definition {
                    bytes += layers.retained_bytes() as u64;
                }
            }
        }
        bytes
    }

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
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.definition.clone()))
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let operation = if let Some(geometry) = self.coordinates() {
            let code = format!(
                "{}\n{}",
                self.function.code,
                include_str!("camera_adjustments.wgsl")
            );
            let mut function = self.function.clone();
            function.bindings.push((
                5,
                FunctionBinding::Uniform(
                    bytemuck::bytes_of(&crate::gpu::flat::FieldGeometry::uncorrected(Some(
                        geometry,
                    )))
                    .into(),
                ),
            ));
            value::Operation::new(
                "camera-coordinate grading v1",
                self.definition.clone(),
                code,
                "camera_adjustments",
                value::Access::Pointwise,
            )
            .with_bindings(function.bindings)
        } else {
            value::Operation::pointwise_function(
                "photographic grading v1",
                self.definition.clone(),
                self.function.clone(),
            )
        };
        value::Calculation::new(domain, output, vec![inputs[0].request(output)?], operation)
            .map(Some)
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
        for coordinates in [
            None,
            Some(crate::gpu::source::SourceGeometry::new(
                [0, 0, 1, 1],
                [1, 1],
            )?),
        ] {
            for active_layers in [false, true] {
                let mut grading = grade
                    .for_grid(domain.grid.coverage)
                    .with_camera_coordinates(coordinates);
                grading.active_layers = active_layers;
                context.prewarm_image(grading.compose(borrowed_input(domain))?)?;
                if active_layers {
                    context.prewarm_image(grading.layers(borrowed_input(Domain {
                        meaning: Meaning::Graded,
                        ..domain
                    }))?)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn shader() -> String {
        [
            crate::gpu::photograph::shader::functions(),
            include_str!("view_bindings.wgsl").to_owned(),
            include_str!("view.wgsl").to_owned(),
        ]
        .join("\n")
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

    pub fn roll(&self, input: Value) -> Result<Value, Error> {
        if input.domain().meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter("Roll requires neutral AP1"));
        }
        Ok(Value::from_operation(
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

    pub fn layers(&self, input: Value) -> Result<Value, Error> {
        if input.domain().meaning != Meaning::Graded {
            return Err(Error::InvalidParameter(
                "adjustment layers require graded AP1",
            ));
        }
        Ok(if self.active_layers {
            Value::from_operation(
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

    pub fn compose(&self, input: Value) -> Result<Value, Error> {
        self.layers(self.roll(input)?)
    }
}
