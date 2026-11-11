//! Compile an ordinary GPU value DAG to bounded execution steps and reused scratch slots.
use super::calculation::Id;
use super::*;
use crate::gpu::evaluation::Access as CompilerAccess;
use std::collections::{HashMap, HashSet};
mod ownership;
pub(crate) use ownership::retire;
use ownership::{Nodes, owned_nodes};

// Bound unit-local compiler descriptors separately from the size of photographic images.
pub(crate) const MAX_UNIT_VALUES: usize = 16_384;

/// A bounded function representation of the same constituent DAG used for stored units.
pub(in crate::gpu::evaluation) struct FunctionGraph {
    root: Id,
    nodes: Nodes,
    input: Option<(Value, CellRegion)>,
    regions: HashMap<Id, CellRegion>,
}
impl FunctionGraph {
    pub(in crate::gpu::evaluation) fn new(
        calculation: &Calculation,
    ) -> Result<Option<Self>, Error> {
        let Some(boundaries) = constituents(calculation)? else {
            return Ok(None);
        };
        let boundary = boundaries
            .iter()
            .enumerate()
            .map(|(i, value)| (Id::Value(value.pointer()), i))
            .collect::<HashMap<_, _>>();
        let (_, root, nodes) = owned_nodes(calculation.clone(), &boundary);
        let regions = required_regions(&nodes, &boundary, root, calculation.coverage())?;
        // Count expression occurrences as well as definitions: shared branches must
        // not turn a small DAG into an exponentially expanded shader.
        const MAX_FUNCTION_VALUES: usize = 64;
        if nodes.len() > MAX_FUNCTION_VALUES
            || !nodes.iter().any(|value| value.shape() == Shape::Pair)
        {
            return Ok(None);
        }
        let mut costs = HashMap::new();
        let mut work = HashMap::<Id, u64>::new();
        let mut input: Option<(Value, CellRegion)> = None;
        let mut reads = Vec::new();
        for node in nodes.iter().rev() {
            let Some(primitive) = node.primitive() else {
                return Ok(None);
            };
            if !matches!(primitive.operation.body.as_ref(), Body::Gpu(_)) {
                return Ok(None);
            }
            let Body::Gpu(body) = primitive.operation.body.as_ref() else {
                unreachable!()
            };
            let mut child_work = 0u64;
            let mut cost = 1usize;
            for (index, read) in primitive
                .read_regions(regions[&node.pointer()])
                .into_iter()
                .enumerate()
            {
                let parent = &node.inputs()[index];
                if boundary.contains_key(&parent.pointer()) {
                    if matches!(primitive.read_access(index), CompilerAccess::Raster) {
                        return Ok(None);
                    }
                    if input.as_ref().is_some_and(|(prior, _)| {
                        !parent
                            .value()
                            .is_some_and(|value| prior.same_capture(value))
                    }) {
                        return Ok(None);
                    }
                    input = Some((parent.value().unwrap().clone(), read));
                    reads.push(read);
                } else {
                    cost = cost.saturating_add(costs[&parent.pointer()]);
                    child_work = child_work.saturating_add(work[&parent.pointer()]);
                }
            }
            if cost > MAX_FUNCTION_VALUES {
                return Ok(None);
            }
            let sampling_work = body
                .sampling_work
                .saturating_mul(child_work.saturating_add(1));
            if sampling_work > 65_536 {
                return Ok(None);
            }
            work.insert(node.pointer(), sampling_work);
            costs.insert(node.pointer(), cost);
        }
        if let Some((_, region)) = &mut input {
            let Some(merged) = rectangular_reads(reads) else {
                return Ok(None);
            };
            if merged.validate().is_err() || merged.width.checked_mul(merged.height).is_none() {
                return Ok(None);
            }
            *region = merged;
        }
        Ok(Some(Self {
            root,
            nodes,
            input,
            regions,
        }))
    }
    fn uniforms(&self) -> Vec<Vec<u8>> {
        let mut uniforms = Vec::new();
        for node in self.nodes.iter() {
            let primitive = node.primitive().unwrap();
            let Body::Gpu(body) = primitive.operation.body.as_ref() else {
                unreachable!()
            };
            let geometry = bytemuck::bytes_of(&compiler::Geometry::new(
                node.domain(),
                self.regions[&node.pointer()],
            ))
            .to_vec();
            if !uniforms.contains(&geometry) {
                uniforms.push(geometry);
            }
            let own = body.uniform.iter().map(|(_, bytes)| bytes.as_ref()).chain(
                body.bindings.iter().filter_map(|(_, binding)| {
                    if let FunctionBinding::Uniform(bytes) = binding {
                        Some(bytes.as_ref())
                    } else {
                        None
                    }
                }),
            );
            for bytes in own {
                if !uniforms.iter().any(|prior| prior.as_slice() == bytes) {
                    uniforms.push(bytes.to_vec());
                }
            }
        }
        uniforms
    }
    pub(in crate::gpu::evaluation) fn may_fuse(&self, limits: Option<&wgpu::Limits>) -> bool {
        // Linking nested sampler libraries grows sharply even for small DAGs.
        // Larger expressions keep their admitted recorded/worker steps.
        if self.nodes.len() > 8 {
            return false;
        }
        self.uniforms().len()
            <= limits.map_or(
                wgpu::Limits::default().max_uniform_buffers_per_shader_stage,
                |limits| limits.max_uniform_buffers_per_shader_stage,
            ) as usize
    }
    pub(in crate::gpu::evaluation) fn emit(
        self,
        fragment: &mut Fragment,
        mut source: impl FnMut(&Value, CellRegion, &mut Fragment) -> Result<Sampled, Error>,
    ) -> Result<Sampled, Error> {
        let input = self
            .input
            .as_ref()
            .map(|(value, region)| source(value, *region, fragment))
            .transpose()?;
        let mut functions = HashMap::<Id, Sampled>::new();
        for node in self.nodes.iter().rev() {
            let primitive = node.primitive().unwrap();
            let inputs = node
                .inputs()
                .iter()
                .map(|parent| {
                    SampleInput::Callable(
                        if self.input.as_ref().is_some_and(|(value, _)| {
                            parent
                                .value()
                                .is_some_and(|parent| value.same_capture(parent))
                        }) {
                            input.as_ref().unwrap().clone()
                        } else {
                            functions[&parent.pointer()].clone()
                        },
                    )
                })
                .collect();
            let sample = primitive.emit(
                fragment,
                inputs,
                node.domain(),
                self.regions[&node.pointer()],
            )?;
            functions.insert(node.pointer(), sample);
        }
        Ok(functions.remove(&self.root).unwrap())
    }
}

// Coalesce only unions known to remain rectangles. A bounding box must not
// acquire holes in sparse dependencies merely to offer callable compilation.
fn rectangular_reads(mut reads: Vec<CellRegion>) -> Option<CellRegion> {
    loop {
        let mut merged = false;
        'pairs: for a in 0..reads.len() {
            for b in a + 1..reads.len() {
                let (left, right) = (reads[a], reads[b]);
                let vertical = left.x == right.x
                    && left.width == right.width
                    && left.y <= right.bottom()
                    && right.y <= left.bottom();
                let horizontal = left.y == right.y
                    && left.height == right.height
                    && left.x <= right.right()
                    && right.x <= left.right();
                if left.contains_region(right)
                    || right.contains_region(left)
                    || vertical
                    || horizontal
                {
                    let x = left.x.min(right.x);
                    let y = left.y.min(right.y);
                    reads[a] = CellRegion::new(
                        [x, y],
                        [
                            (i64::from(left.right().max(right.right())) - i64::from(x)) as u32,
                            (i64::from(left.bottom().max(right.bottom())) - i64::from(y)) as u32,
                        ],
                    );
                    reads.swap_remove(b);
                    merged = true;
                    break 'pairs;
                }
            }
        }
        if !merged {
            return (reads.len() == 1).then(|| reads[0]);
        }
    }
}

#[cfg(all(test, feature = "editor"))]
mod function_tests {
    use super::*;

    #[test]
    fn function_input_coalescing_preserves_rectangles_without_acquiring_holes() {
        let full = CellRegion::new([-2, -3], [4, 4]);
        let parts = [
            CellRegion::new([-2, -3], [2, 2]),
            CellRegion::new([0, -3], [2, 2]),
            CellRegion::new([-2, -1], [2, 2]),
            CellRegion::new([0, -1], [2, 2]),
        ];
        for offset in 0..parts.len() {
            let mut shifted = parts.to_vec();
            shifted.rotate_left(offset);
            assert_eq!(rectangular_reads(shifted.clone()), Some(full));
            shifted.reverse();
            assert_eq!(rectangular_reads(shifted), Some(full));
        }
        assert_eq!(
            rectangular_reads(vec![full, parts[0], parts[3]]),
            Some(full)
        );
        assert_eq!(
            rectangular_reads(vec![parts[0], parts[3]]),
            None,
            "diagonal support has holes; its bounding box is not a source request"
        );
    }
}

fn required_regions(
    nodes: &Nodes,
    boundary: &HashMap<Id, usize>,
    root_pointer: Id,
    region: CellRegion,
) -> Result<HashMap<Id, CellRegion>, Error> {
    let mut regions = HashMap::from([(root_pointer, region)]);
    for node in nodes.iter() {
        let primitive = node.primitive().ok_or(Error::InvalidParameter(
            "GPU value graph contains an undeclared boundary",
        ))?;
        let region = primitive.output_region(regions[&node.pointer()]);
        if primitive
            .extent
            .is_some_and(|extent| !extent.contains_region(region))
        {
            return Err(Error::InvalidParameter(
                "GPU graph demand exceeds its value",
            ));
        }
        regions.insert(node.pointer(), region);
        for (input, read) in node.inputs().iter().zip(primitive.read_regions(region)) {
            if boundary.contains_key(&input.pointer()) {
                continue;
            }
            regions
                .entry(input.pointer())
                .and_modify(|prior| {
                    let x = prior.x.min(read.x);
                    let y = prior.y.min(read.y);
                    *prior = CellRegion::new(
                        [x, y],
                        [
                            (prior.right().max(read.right()) as i64 - i64::from(x)) as u32,
                            (prior.bottom().max(read.bottom()) as i64 - i64::from(y)) as u32,
                        ],
                    );
                })
                .or_insert(read);
        }
    }
    Ok(regions)
}

struct Input {
    image: PlannedImage,
    binding: UnitValue,
}
struct Step {
    primitive: Arc<compiler::Primitive>,
    output: PlannedImage,
    inputs: Vec<Input>,
}

/// Boundaries remain ordinary upstream image requests; constituent values are unit-local.
/// The common scheduler assigns their live scratch, and the existing worker executes it.
pub(crate) fn compile(root: Calculation, boundaries: &[Value]) -> Result<GpuUnit, Error> {
    let boundary = boundaries
        .iter()
        .enumerate()
        .map(|(i, node)| (Id::Value(node.pointer()), i))
        .collect::<HashMap<_, _>>();
    let region = root.coverage();
    let (shape, root_pointer, nodes) = owned_nodes(root, &boundary);
    let mut fragment = Fragment::default();
    let mut outputs = HashMap::<Id, PlannedImage>::new();
    let mut steps = Vec::new();
    if nodes.len() > MAX_UNIT_VALUES {
        return Err(Error::WorkingAllowance(
            "calculation exceeds the unit graph capacity",
        ));
    }
    if shape != Shape::Pixel {
        return Err(Error::InvalidParameter(
            "GPU image graph must return photographic pixels",
        ));
    }
    let regions = required_regions(&nodes, &boundary, root_pointer, region)?;
    for node in nodes.iter().rev() {
        let primitive = node.primitive().cloned().ok_or(Error::InvalidParameter(
            "GPU value graph contains an undeclared boundary",
        ))?;
        if let Body::Failed(error) = primitive.operation.body.as_ref() {
            return Err(error.clone());
        }
        if matches!(primitive.operation.body.as_ref(), Body::Cpu(_)) {
            return Err(Error::InvalidParameter(
                "CPU values must be upstream GPU graph boundaries",
            ));
        }
        let region = regions[&node.pointer()];
        let cells = region
            .width
            .checked_mul(region.height)
            .ok_or(Error::WorkingAllowance(
                "unit state exceeds GPU cell addressing",
            ))?;
        let bytes = u64::from(cells) * primitive.shape().bytes();
        let resource = fragment.resource(if node.pointer() == root_pointer {
            Storage::Input {
                slot: 0,
                bytes,
                writable: true,
            }
        } else {
            Storage::Buffer(bytes)
        });
        let output = PlannedImage {
            domain: node.domain(),
            region,
            resource,
        };
        let mut inputs = Vec::new();
        let mut bindings = Vec::new();
        for (index, read) in primitive.read_regions(region).iter().enumerate() {
            let parent = &node.inputs()[index];
            let key = parent.pointer();
            if let Some(&input) = boundary.get(&key) {
                inputs.push(Input {
                    image: PlannedImage {
                        domain: parent.domain(),
                        region: *read,
                        resource: Resource(0),
                    },
                    binding: UnitValue::Read(input),
                });
            } else {
                let image = *outputs
                    .get(&key)
                    .ok_or(Error::InvalidParameter("GPU graph input was not scheduled"))?;
                if !image.region.contains_region(*read) {
                    return Err(Error::InvalidParameter("GPU graph read exceeds its value"));
                }
                bindings.push((index as u32, Binding::Read(image.resource)));
                inputs.push(Input {
                    image,
                    binding: UnitValue::State(image.resource.0),
                });
            }
        }
        bindings.push((inputs.len() as u32, Binding::Write(resource)));
        fragment.compute("", "value_lifetime", bindings, 1);
        outputs.insert(node.pointer(), output);
        steps.push(Step {
            primitive,
            output,
            inputs,
        });
    }
    let output = *outputs
        .get(&root_pointer)
        .ok_or(Error::InvalidParameter("GPU graph has no output"))?;
    let scheduled = super::super::plan::schedule(fragment, output)?;
    let output_slot = scheduled.assignment[output.resource.0];
    let mut states = Vec::new();
    let mut slots = Vec::new();
    for (index, storage) in scheduled.slots.iter().enumerate() {
        if index == output_slot {
            slots.push(UnitValue::Output);
        } else {
            slots.push(UnitValue::State(states.len()));
            states.push(storage.bytes());
        }
    }
    let mut phases = Vec::with_capacity(steps.len());
    for step in steps {
        let mut reads = Vec::new();
        let mut bindings = Vec::new();
        for input in &step.inputs {
            bindings.push(match input.binding {
                UnitValue::Read(index) => {
                    let slot = reads.len();
                    reads.push(Read::new(index, input.image.region));
                    UnitValue::Read(slot)
                }
                UnitValue::State(resource) => slots[scheduled.assignment[resource]],
                UnitValue::Output => unreachable!(),
            });
        }
        bindings.push(slots[scheduled.assignment[step.output.resource.0]]);
        phases.push(Ok(GpuPhase {
            reads,
            inputs: bindings,
            kernel: Arc::new(Kernel {
                primitive: step.primitive,
                output: step.output,
                inputs: step.inputs.into_iter().map(|input| input.image).collect(),
            }),
        }));
    }
    drop(nodes);
    Ok(GpuUnit {
        state: states,
        phases: Box::new(phases.into_iter()),
    })
}

pub(in crate::gpu::evaluation) struct Kernel {
    pub primitive: Arc<compiler::Primitive>,
    pub output: PlannedImage,
    pub inputs: Vec<PlannedImage>,
}
impl Kernel {
    pub(in crate::gpu::evaluation) fn can_sample(&self, input: usize) -> bool {
        matches!(self.primitive.operation.body.as_ref(), Body::Gpu(_))
            && matches!(self.primitive.read_access(input), CompilerAccess::Callable)
    }
    pub(in crate::gpu::evaluation) fn lower_samples(
        &self,
        fragment: &mut Fragment,
        inputs: Vec<SampleInput>,
        resource: Resource,
    ) -> Result<PlannedImage, Error> {
        let output = PlannedImage {
            resource,
            ..self.output
        };
        self.primitive
            .emit(fragment, inputs, output.domain, output.region)?
            .materialize_type(fragment, output, self.primitive.shape().wgsl())?;
        Ok(output)
    }
}
impl GpuKernel for Kernel {
    fn working_bytes(&self) -> u64 {
        self.primitive.working_bytes()
    }
    fn lower(&self, fragment: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, Error> {
        if slots.len() != self.inputs.len() + 1 {
            return Err(Error::InvalidParameter(
                "GPU value graph bindings differ from their declaration",
            ));
        }
        let inputs = self
            .inputs
            .iter()
            .zip(slots)
            .map(|(input, resource)| PlannedImage {
                resource: *resource,
                ..*input
            })
            .collect::<Vec<_>>();
        let output = PlannedImage {
            resource: slots[self.inputs.len()],
            ..self.output
        };
        self.primitive.lower(fragment, &inputs, output)?;
        Ok(output)
    }
}
