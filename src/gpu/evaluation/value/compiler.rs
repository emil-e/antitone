//! GPU kernel ABI and demanded layouts, separate from captured mathematical definitions.
use super::*;
use crate::gpu::evaluation::Access as CompilerAccess;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Geometry {
    origin: [f32; 2],
    cell: [i32; 2],
    axes: [[f32; 2]; 2],
}

impl Geometry {
    pub(super) fn new(domain: Domain, region: CellRegion) -> Self {
        let origin = domain.point([f64::from(region.x) + 0.5, f64::from(region.y) + 0.5]);
        Self {
            origin: origin.map(|value| value as f32),
            cell: [region.x, region.y],
            axes: domain.grid.axes.map(|axis| axis.map(|value| value as f32)),
        }
    }
}

pub(in crate::gpu::evaluation) struct Primitive {
    pub operation: Operation,
    pub reads: Vec<CellRegion>,
    pub coverage: CellRegion,
    pub extent: Option<CellRegion>,
    pub raster_inputs: Vec<bool>,
    pub input_shapes: Vec<Shape>,
    same_position_copy: bool,
}

pub(super) struct Input {
    pub domain: Domain,
    pub coverage: CellRegion,
    pub access: Access,
    pub shape: Shape,
}
impl Input {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn coverage(&self) -> CellRegion {
        self.coverage
    }
    fn shape(&self) -> Shape {
        self.shape
    }
}
impl Primitive {
    pub(super) fn new(
        domain: Domain,
        coverage: CellRegion,
        inputs: &[Input],
        operation: Operation,
    ) -> Result<(Self, Access), Error> {
        if !domain.contains_region(coverage.validate()?) {
            return Err(Error::InvalidParameter(
                "value coverage exceeds its image domain",
            ));
        }
        if let Body::Gpu(body) = operation.body.as_ref()
            && !body.input_access.is_empty()
            && body.input_access.len() != inputs.len()
        {
            return Err(Error::InvalidParameter(
                "pixel input access differs from its inputs",
            ));
        }
        let same_position_copy = matches!(operation.body.as_ref(), Body::Concatenate)
            && inputs.len() == 1
            && inputs[0].coverage() == coverage
            && inputs[0].domain().grid == domain.grid;
        let input_access = |index| {
            if same_position_copy {
                Access::Pointwise
            } else {
                operation.input_access(index)
            }
        };
        if inputs.iter().enumerate().any(|(index, input)| {
            input_access(index) == Access::Pointwise
                && (input.domain().grid != domain.grid
                    || !input.coverage().contains_region(coverage))
        }) {
            return Err(Error::InvalidParameter(
                "pointwise input cannot supply the output coordinates",
            ));
        }
        let spatial_input = inputs
            .iter()
            .any(|input| input.access == Access::Neighborhood);
        let raster_inputs = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                input_access(index) == Access::Neighborhood && input.access == Access::Neighborhood
            })
            .collect();
        let access = if spatial_input {
            Access::Neighborhood
        } else if same_position_copy {
            Access::Pointwise
        } else {
            operation.access()
        };
        let reads = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                if input_access(index) == Access::Pointwise {
                    coverage
                } else {
                    input.coverage()
                }
            })
            .collect::<Vec<_>>();
        if matches!(operation.body.as_ref(), Body::Gpu(_))
            && reads
                .iter()
                .any(|r| r.width.checked_mul(r.height).is_none())
        {
            return Err(Error::InvalidParameter(
                "evaluation raster exceeds GPU cell addressing",
            ));
        }
        let input_shapes = inputs.iter().map(Input::shape).collect();

        Ok((
            Self {
                operation,
                reads,
                coverage,
                extent: Some(coverage),
                raster_inputs,
                input_shapes,
                same_position_copy,
            },
            access,
        ))
    }
}

impl Primitive {
    pub(in crate::gpu::evaluation) fn working_bytes(&self) -> u64 {
        let Body::Gpu(body) = self.operation.body.as_ref() else {
            return 0;
        };
        80 + self.reads.len() as u64 * 16
            + body
                .uniform
                .as_ref()
                .map_or(0, |(_, bytes)| bytes.len() as u64)
            + body
                .tables
                .iter()
                .map(|rows| rows.len() as u64 * 16)
                .sum::<u64>()
            + body
                .bindings
                .iter()
                .map(|(_, binding)| match binding {
                    FunctionBinding::Shared(_) => 0,
                    FunctionBinding::Uniform(bytes) | FunctionBinding::Storage(bytes) => {
                        bytes.len() as u64
                    }
                })
                .sum::<u64>()
    }
    pub(super) fn output_region(&self, requested: CellRegion) -> CellRegion {
        if matches!(self.operation.body.as_ref(), Body::Concatenate) && !self.same_position_copy() {
            self.coverage
        } else {
            requested
        }
    }

    pub(super) fn read_regions(&self, output: CellRegion) -> Vec<CellRegion> {
        self.reads
            .iter()
            .enumerate()
            .map(|(index, region)| {
                if self.same_position_copy()
                    || self.operation.input_access(index) == Access::Pointwise
                {
                    output
                } else {
                    *region
                }
            })
            .collect()
    }

    fn same_position_copy(&self) -> bool {
        self.same_position_copy && self.input_shapes.as_slice() == [Shape::Pixel]
    }
    pub(super) fn shape(&self) -> Shape {
        match self.operation.body.as_ref() {
            Body::Gpu(body) => body.shape,
            Body::Source(_)
            | Body::Argument { .. }
            | Body::Cpu(_)
            | Body::Concatenate
            | Body::Failed(_) => Shape::Pixel,
        }
    }
}

impl Primitive {
    pub(in crate::gpu::evaluation) fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(self
            .read_regions(output)
            .iter()
            .enumerate()
            .map(|(input, region)| Read::new(input, *region))
            .collect())
    }

    pub(in crate::gpu::evaluation) fn plan_unit(
        self: Arc<Self>,
        upstream: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<WorkUnit, Error> {
        match self.operation.body.as_ref() {
            Body::Source(source) => Ok(WorkUnit {
                inputs: upstream.to_vec(),
                domain,
                output,
                work: UnitWork::Source(source.clone()),
            }),
            Body::Cpu(kernel) => Ok(WorkUnit {
                inputs: upstream.to_vec(),
                domain,
                output: self.coverage,
                work: UnitWork::Cpu(CpuUnit {
                    reads: self.reads(output)?,
                    kernel: kernel.clone(),
                }),
            }),
            Body::Failed(error) => Err(error.clone()),
            Body::Gpu(_) | Body::Argument { .. } | Body::Concatenate => {
                let reads = self.reads(output)?;
                let images = reads
                    .iter()
                    .map(|read| PlannedImage {
                        domain: upstream[read.input].domain(),
                        region: read.region,
                        resource: Resource(0),
                    })
                    .collect();
                let mut inputs = (0..reads.len()).map(UnitValue::Read).collect::<Vec<_>>();
                inputs.push(UnitValue::Output);
                Ok(WorkUnit {
                    inputs: upstream.to_vec(),
                    domain,
                    output,
                    work: UnitWork::Gpu(GpuUnit {
                        state: Vec::new(),
                        phases: Box::new(std::iter::once(Ok(GpuPhase {
                            reads,
                            inputs,
                            kernel: Arc::new(super::graph::Kernel {
                                primitive: self,
                                output: PlannedImage {
                                    domain,
                                    region: output,
                                    resource: Resource(0),
                                },
                                inputs: images,
                            }),
                        }))),
                    }),
                })
            }
        }
    }

    pub(in crate::gpu::evaluation) fn lower(
        &self,
        fragment: &mut Fragment,
        inputs: &[PlannedImage],
        output: PlannedImage,
    ) -> Result<(), Error> {
        if let Body::Argument { writable } = self.operation.body.as_ref() {
            let bounds = output.domain.finite()?;
            let bytes = u64::from(bounds.width) * u64::from(bounds.height) * 16;
            let storage = Storage::Input {
                slot: 0,
                bytes,
                writable: *writable,
            };
            if output.region == bounds {
                fragment.resources[output.resource.0] = storage;
                return Ok(());
            }
            let source = fragment.resource(storage);
            // External arguments promise STORAGE, not transfer usages. Reinterpret
            // lanes as integers so physical gathering preserves every sample bit.
            return Sampled::buffer_type(fragment, bounds, Binding::Read(source), "vec4u")
                .materialize_type(fragment, output, "vec4u");
        }
        if let Body::Source(source) = self.operation.body.as_ref() {
            fragment.work.push(Work::Upload {
                source: source.clone(),
                output,
            });
            return Ok(());
        }
        if matches!(self.operation.body.as_ref(), Body::Concatenate) {
            if self.same_position_copy() && inputs[0].region != output.region {
                return Sampled::buffer_type(
                    fragment,
                    inputs[0].region,
                    Binding::Read(inputs[0].resource),
                    "vec4u",
                )
                .materialize_type(fragment, output, "vec4u");
            }
            let mut offset = 0u64;
            for (input, read) in inputs.iter().zip(self.read_regions(output.region)) {
                if !input.region.contains_region(read) {
                    return Err(Error::InvalidParameter(
                        "concatenation read exceeds its value",
                    ));
                }
                if read == input.region {
                    let bytes = u64::from(read.width) * u64::from(read.height) * 16;
                    fragment.work.push(Work::Copy {
                        source: input.resource,
                        target: output.resource,
                        source_offset: 0,
                        target_offset: offset,
                        bytes,
                    });
                    offset += bytes;
                    continue;
                }
                if u64::from(read.height) + fragment.work.len() as u64 > MAX_UNIT_VALUES as u64 {
                    return Err(Error::WorkingAllowance(
                        "calculation exceeds the unit graph capacity",
                    ));
                }
                for y in read.y..read.bottom() {
                    let bytes = u64::from(read.width) * 16;
                    fragment.work.push(Work::Copy {
                        source: input.resource,
                        target: output.resource,
                        source_offset: input.region.index([read.x, y]) as u64 * 16,
                        target_offset: offset,
                        bytes,
                    });
                    offset += bytes;
                }
            }
            if offset != u64::from(output.cells()) * 16 {
                return Err(Error::InvalidParameter(
                    "concatenated arguments differ from their output",
                ));
            }
            return Ok(());
        }
        let inputs = inputs
            .iter()
            .zip(&self.input_shapes)
            .map(|(input, shape)| {
                Sampled::buffer_type(
                    fragment,
                    input.region,
                    Binding::Read(input.resource),
                    shape.wgsl(),
                )
            })
            .collect();
        self.link(fragment, inputs, output.domain, output.region)?
            .materialize_type(fragment, output, self.shape().wgsl())
    }
}

impl Primitive {
    pub(in crate::gpu::evaluation) fn can_sample(&self) -> bool {
        matches!(
            self.operation.body.as_ref(),
            Body::Gpu(_) | Body::Argument { .. }
        ) || self.same_position_copy()
    }
    pub(in crate::gpu::evaluation) fn access(&self) -> CompilerAccess {
        if !self.same_position_copy()
            && !self.raster_inputs.is_empty()
            && self.raster_inputs.iter().all(|raster| *raster)
        {
            CompilerAccess::Raster
        } else {
            CompilerAccess::Callable
        }
    }

    pub(in crate::gpu::evaluation) fn read_access(&self, input: usize) -> CompilerAccess {
        if self.raster_inputs[input] {
            CompilerAccess::Raster
        } else {
            CompilerAccess::Callable
        }
    }

    pub(in crate::gpu::evaluation) fn emit(
        &self,
        fragment: &mut Fragment,
        inputs: Vec<SampleInput>,
        domain: Domain,
        region: CellRegion,
    ) -> Result<Sampled, Error> {
        let inputs = inputs
            .into_iter()
            .zip(&self.input_shapes)
            .map(|(input, shape)| match input {
                SampleInput::Callable(input) => input,
                SampleInput::Raster(input) => Sampled::buffer_type(
                    fragment,
                    input.region,
                    Binding::Read(input.resource),
                    shape.wgsl(),
                ),
            })
            .collect();
        self.link(fragment, inputs, domain, region)
    }
}

impl Primitive {
    fn link(
        &self,
        fragment: &mut Fragment,
        mut inputs: Vec<Sampled>,
        domain: Domain,
        region: CellRegion,
    ) -> Result<Sampled, Error> {
        if self.same_position_copy() {
            return inputs
                .pop()
                .filter(|_| inputs.is_empty())
                .ok_or(Error::InvalidParameter("image window requires one input"));
        }
        match self.operation.body.as_ref() {
            Body::Argument { writable } => {
                let bounds = domain.finite()?;
                let input = fragment.resource(Storage::Input {
                    slot: 0,
                    bytes: u64::from(bounds.width) * u64::from(bounds.height) * 16,
                    writable: *writable,
                });
                Ok(Sampled::buffer(fragment, bounds, Binding::Read(input)))
            }
            Body::Gpu(body) => body.link(fragment, inputs, &self.input_shapes, domain, region),
            Body::Failed(error) => Err(error.clone()),
            Body::Source(_) | Body::Cpu(_) | Body::Concatenate => Err(Error::InvalidParameter(
                "CPU calculation cannot be lowered to WGSL",
            )),
        }
    }
}

impl GpuBody {
    fn link(
        &self,
        fragment: &mut Fragment,
        inputs: Vec<Sampled>,
        input_shapes: &[Shape],
        domain: Domain,
        region: CellRegion,
    ) -> Result<Sampled, Error> {
        // Validate only during lowering: requesting a value must not parse/lower WGSL.
        let access = |i: usize| self.input_access.get(i).copied().unwrap_or(self.access);
        let mut stubs = (0..inputs.len())
            .filter(|i| access(*i) == Access::Neighborhood)
            .map(|i| {
                format!(
                    "fn input_{i}(p: vec2i) -> {} {{ var empty: {}; return empty; }}\n",
                    input_shapes[i].wgsl(),
                    input_shapes[i].wgsl()
                )
            })
            .collect::<String>();
        for i in 0..self.tables.len() {
            stubs.push_str(&format!(
                "fn table_{i}(index: u32) -> vec4u {{ return vec4u(0u); }}\n"
            ));
        }
        let library = fragment.library(&format!("{stubs}\n{}", self.code))?;
        let module = &library.module;
        if library.authored_entries
            || module.global_variables.iter().any(|(_, global)| {
                global
                    .binding
                    .as_ref()
                    .is_none_or(|binding| binding.group != 0)
            })
        {
            return Err(Error::InvalidParameter(
                "pixel bodies cannot declare storage or dispatch entry points",
            ));
        }
        if !module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some(self.entry))
        {
            return Err(Error::InvalidParameter(
                "pixel function is absent from its module",
            ));
        }
        let used = library.resources(&[self.entry])?;
        let mut bindings = Vec::new();
        for slot in used {
            let argument = self
                .bindings
                .iter()
                .find(|(index, _)| *index == slot)
                .ok_or(Error::InvalidParameter(
                    "pixel bodies cannot declare storage or dispatch entry points",
                ))?;
            bindings.push((slot, argument.1.lower(fragment)));
        }
        for (slot, _) in &self.bindings {
            if self
                .bindings
                .iter()
                .filter(|(index, _)| index == slot)
                .count()
                != 1
                || !module.global_variables.iter().any(|(_, global)| {
                    global
                        .binding
                        .as_ref()
                        .is_some_and(|binding| binding.binding == *slot)
                })
            {
                return Err(Error::InvalidParameter(
                    "pixel capture differs from its shader resources",
                ));
            }
        }
        let mut slot = module
            .global_variables
            .iter()
            .filter_map(|(_, global)| global.binding.as_ref().map(|binding| binding.binding))
            .max()
            .unwrap_or(63)
            .max(63)
            .checked_add(1)
            .ok_or(Error::InvalidParameter(
                "pixel resource slots exceed GPU addressing",
            ))?;
        let mut code = String::new();
        let mut pixel_arguments = String::new();
        for (i, input) in inputs.into_iter().enumerate() {
            let input = input.namespaced(fragment, &format!("value_input_{i}_"), &mut slot)?;
            code.push_str(&input.code);
            if access(i) == Access::Neighborhood {
                code.push_str(&format!(
                    "\nfn input_{i}(p: vec2i) -> {} {{ return {}(p); }}\n",
                    input_shapes[i].wgsl(),
                    input.symbol
                ));
            } else {
                pixel_arguments.push_str(&format!(", {}(p)", input.symbol));
            }
            bindings.extend(input.bindings);
        }
        for (i, rows) in self.tables.iter().enumerate() {
            if rows.is_empty() {
                return Err(Error::InvalidParameter("pixel table is empty"));
            }
            code.push_str(&format!(
                "\n@group(0) @binding({slot}) var<storage, read> value_table_{i}: array<vec4u>;\nfn table_{i}(index: u32) -> vec4u {{ return value_table_{i}[index]; }}\n"
            ));
            let data = fragment.resource(Storage::Data(bytemuck::cast_slice(rows).into()));
            bindings.push((slot, Binding::Read(data)));
            slot += 1;
        }
        code.push_str(&self.code);
        let uniform_argument = if let Some((ty, bytes)) = &self.uniform {
            if bytes.is_empty() || bytes.len() % 16 != 0 {
                return Err(Error::InvalidParameter(
                    "pixel uniform must contain complete 16-byte rows",
                ));
            }
            code.push_str(&format!(
                "\n@group(0) @binding({slot}) var<uniform> value_parameters: {ty};\n"
            ));
            bindings.push((
                slot,
                Binding::Uniform(fragment.resource(Storage::Uniform(bytes.clone()))),
            ));
            slot += 1;
            ", value_parameters"
        } else {
            ""
        };
        let position = if library.argument_used(self.entry, 1)? {
            let geometry = fragment.uniform(&Geometry::new(domain, region));
            code.push_str(&format!(
                "\n@group(0) @binding({slot}) var<uniform> value_geometry: array<vec4u, 2>;\n"
            ));
            bindings.push((slot, geometry));
            "let local = vec2f(p - bitcast<vec2i>(value_geometry[0].zw));\nlet position = bitcast<vec2f>(value_geometry[0].xy) + local.x * bitcast<vec2f>(value_geometry[1].xy) + local.y * bitcast<vec2f>(value_geometry[1].zw);"
        } else {
            "let position = vec2f(0.0);"
        };
        code.push_str(&format!(
            r#"
fn value_sample(p: vec2i) -> {result_type} {{
    {position}
    return {}(p, position{pixel_arguments}{uniform_argument});
}}
"#,
            self.entry,
            result_type = self.shape.wgsl()
        ));
        let sample = Sampled {
            code: code.into(),
            symbol: "value_sample".into(),
            bindings,
        };
        sample.namespaced(fragment, "pixel_", &mut slot)
    }
}
