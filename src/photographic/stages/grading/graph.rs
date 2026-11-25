//! Captured photographic functions over neutral AP1, independent of raster acquisition.

use super::Grading;
use crate::Error;
use crate::evaluation::*;

fn grading_context(
    parameters: &crate::photographic::parameters::ProcessingParameters,
    coverage: u32,
) -> [[u32; 4]; 2] {
    // Match the filtered-view gain packing rather than changing CPU/GPU rounding.
    [
        [parameters.anchor.w.exp().to_bits(), 0, 0, 0],
        [coverage, 0, 0, 0],
    ]
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

    fn layers_function(&self) -> Function {
        Function::new(Grading::shader(), "evaluation_adjustments")
            .with_storage(6, self.layers.clone())
            .with_uniform(7, &self.context)
            .with_uniform(8, &grading_context(&self.parameters, self.coverage))
    }
}

struct Grade {
    definition: GradeDefinition,
    function: Function,
}

#[derive(Clone, Debug, PartialEq, Hash)]
enum GradeDefinition {
    Roll(crate::processing::RollSettings, u32),
    Layers {
        layers: crate::adjustment_layers::AdjustmentLayerSnapshot,
        area: crate::geometry::SourceRegion,
        coverage: u32,
        coordinates: Option<crate::acquisition::camera::SourceGeometry>,
    },
}

impl Grade {
    fn coordinates(&self) -> Option<crate::acquisition::camera::SourceGeometry> {
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

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::evaluation::Read>, Error> {
        Ok(vec![crate::evaluation::Read::new(0, region)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::evaluation::Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
        let inputs = crate::evaluation::value::Calculation::references(inputs);
        let operation = if let Some(geometry) = self.coordinates() {
            let code = Grading::camera_shader(&self.function.code);
            let function =
                crate::acquisition::camera::coordinates::bind(self.function.clone(), geometry);
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
    pub(crate) fn warmup_values(
        device: &wgpu::Device,
        domain: Domain,
    ) -> Result<Vec<Value>, Error> {
        let grade = Self::new(
            &device.limits(),
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
            [0, 0, 1, 1].into(),
        )?;
        let mut images = Vec::new();
        for coordinates in [
            None,
            Some(crate::acquisition::camera::SourceGeometry::new(
                [0, 0, 1, 1],
                [1, 1],
            )?),
        ] {
            for active_layers in [false, true] {
                let mut grading = grade
                    .for_grid(domain.grid.coverage)
                    .with_camera_coordinates(coordinates);
                grading.active_layers = active_layers;
                images.push(grading.compose(borrowed_input(domain))?);
                if active_layers {
                    images.push(grading.layers(borrowed_input(domain))?);
                }
            }
        }
        Ok(images)
    }

    pub(crate) fn camera_shader(grading: &str) -> String {
        [
            grading,
            &crate::acquisition::camera::coordinates::shader(),
            include_str!("camera_adjustments.wgsl"),
        ]
        .join("\n")
    }

    pub(crate) fn shader() -> String {
        [
            crate::photographic::stages::shader::functions(),
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
        geometry: Option<crate::acquisition::camera::SourceGeometry>,
    ) -> Self {
        self.coordinates = geometry;
        self
    }

    pub(crate) fn roll_image(
        input: Value,
        roll: crate::processing::RollSettings,
    ) -> Result<Value, Error> {
        let coverage = input.domain().grid.coverage;
        Self::roll_at(input, roll, coverage)
    }

    pub fn roll(&self, input: Value) -> Result<Value, Error> {
        Self::roll_at(input, self.roll, self.coverage)
    }

    fn roll_at(
        input: Value,
        roll: crate::processing::RollSettings,
        coverage: u32,
    ) -> Result<Value, Error> {
        let parameters = crate::photographic::parameters::ProcessingParameters::photographic(roll)?;
        let function = Function::new(Grading::shader(), "evaluation_roll")
            .with_uniform(2, &parameters)
            .with_uniform(8, &grading_context(&parameters, coverage));
        Ok(Value::from_operation(
            input.domain(),
            Some(input),
            Grade {
                definition: GradeDefinition::Roll(roll, coverage),
                function,
            },
        ))
    }

    pub fn layers(&self, input: Value) -> Result<Value, Error> {
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
                    function: self.layers_function(),
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
