//! Evaluation demand and schedule declared resource lifetimes, without operation-specific dispatch.

use super::CellRegion as Rect;
use super::*;

mod required;
mod values;
use required::productions_are_required;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Materialized,
    Composed,
}

/// Per-calculation working allowance, separate from optional cache retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub tile: [u32; 2],
    pub scratch_bytes: u64,
}

pub struct Tile {
    pub fragment: Fragment,
    pub result: PlannedImage,
    pub slots: Vec<Storage>,
    pub assignment: Vec<usize>,
    pub last_use: Vec<usize>,
}

/// Captured, lazy traversal. Only one tile's physical plan is retained during execution.
#[derive(Clone)]
pub struct Job {
    root: value::Value,
    region: Rect,
    budget: Budget,
    policy: Policy,
    materializations: Option<Materializations>,
    supplied: Arc<[SuppliedImage]>,
    limits: Option<wgpu::Limits>,
}

impl Job {
    pub fn new(root: impl Into<value::Value>, region: Rect, budget: Budget) -> Result<Self, Error> {
        let root = root.into();
        region.validate()?;
        if budget.tile.contains(&0) || !root.domain.contains_region(region) {
            return Err(Error::InvalidParameter("invalid evaluation job demand"));
        }

        Ok(Self {
            root,
            region,
            budget,
            policy: Policy::Materialized,
            materializations: None,
            supplied: Arc::from([]),
            limits: None,
        })
    }

    pub(super) fn with_limits(mut self, limits: wgpu::Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    pub(super) fn image(&self) -> &value::Value {
        &self.root
    }
    pub(super) fn materializations(&self) -> Option<&Materializations> {
        self.materializations.as_ref()
    }

    pub(super) fn budget(&self) -> Budget {
        self.budget
    }

    pub(super) fn region(&self) -> Rect {
        self.region
    }

    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    pub(super) fn with_materializations(mut self, scope: Materializations) -> Self {
        self.materializations = Some(scope);
        self
    }

    pub(super) fn with_supplied(mut self, supplied: Arc<[SuppliedImage]>) -> Self {
        self.supplied = supplied;
        self
    }

    pub(super) fn probe(&self, region: Rect) -> Result<Plan, Error> {
        let mut fragment = Fragment::with_programs(
            self.materializations
                .as_ref()
                .map(|scope| scope.programs().clone()),
        );
        let result = evaluate_region(
            &self.root,
            region,
            &mut fragment,
            Planning {
                policy: self.policy,
                scope: self.materializations.as_ref(),
                supplied: &self.supplied,
                scratch_bytes: self.budget.scratch_bytes,
                tile: self.budget.tile,
                limits: self.limits.as_ref(),
            },
        )?;
        Plan::from_work(fragment, result, u64::MAX)
    }

    #[cfg(feature = "render")]
    pub(crate) fn texture_plan(&self, texture: &wgpu::Texture) -> Result<Plan, Error> {
        if self.root.domain.meaning != Meaning::Display
            || texture.format() != wgpu::TextureFormat::Rgba32Float
            || texture.dimension() != wgpu::TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || [texture.width(), texture.height()] != [self.region.width, self.region.height]
            || self.budget.tile[0] < self.region.width
            || self.budget.tile[1] < self.region.height
        {
            return Err(Error::InvalidParameter(
                "display attachment differs from composed image",
            ));
        }
        let planning = Planning {
            policy: self.policy,
            scope: self.materializations.as_ref(),
            supplied: &self.supplied,
            scratch_bytes: self.budget.scratch_bytes,
            tile: self.budget.tile,
            limits: self.limits.as_ref(),
        };
        for callable in [true, false] {
            let mut fragment = Fragment::with_programs(
                self.materializations
                    .as_ref()
                    .map(|scope| scope.programs().clone()),
            );
            let sample = if callable {
                sample_region(&self.root, self.region, &mut fragment, planning)
            } else {
                evaluate_region(&self.root, self.region, &mut fragment, planning)
                    .map(|image| Sampled::raster(&mut fragment, image))
            };
            if let Ok(sample) = sample
                && productions_are_required(&self.root, self.region, &fragment.work, planning)
                    .unwrap_or(false)
            {
                let output = PlannedImage {
                    domain: self.root.domain,
                    region: self.region,
                    resource: fragment.resource(Storage::RenderTarget(texture.clone())),
                };
                sample.render(&mut fragment, output)?;
                if planning.fits(&fragment, output)? {
                    return Plan::from_work(fragment, output, self.budget.scratch_bytes);
                }
            }
        }
        Err(Error::InvalidParameter(
            "display attachment exceeds the working allowance",
        ))
    }

    pub fn plans(&self) -> impl Iterator<Item = Result<Plan, Error>> + '_ {
        let area = self.region;

        (area.y..area.bottom())
            .step_by(self.budget.tile[1] as usize)
            .flat_map(move |y| {
                (area.x..area.right())
                    .step_by(self.budget.tile[0] as usize)
                    .map(move |x| {
                        let r = Rect {
                            x,
                            y,
                            width: self.budget.tile[0].min((area.right() - x) as u32),
                            height: self.budget.tile[1].min((area.bottom() - y) as u32),
                        };
                        Plan::with_materializations(
                            self.root.clone(),
                            r,
                            self.budget,
                            self.policy,
                            self.materializations.as_ref(),
                            &self.supplied,
                            self.limits.as_ref(),
                        )
                    })
            })
    }
}

pub struct Plan {
    pub domain: Domain,
    pub region: Rect,
    pub tiles: Vec<Tile>,
    pub peak_scratch_bytes: u64,
    pub scratch_budget: u64,
}

impl Plan {
    pub(super) fn requires_production(&self, root: &value::Value) -> Result<bool, Error> {
        for work in self.tiles.iter().flat_map(|tile| &tile.fragment.work) {
            if let Work::Produce { image, output } = work
                && image.domain() == root.domain()
                && root.domain().contains_region(output.region)
                && image
                    .output(output.region)?
                    .same_value(&root.output(output.region)?)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn from_work(
        fragment: Fragment,
        result: PlannedImage,
        budget: u64,
    ) -> Result<Self, Error> {
        result.region.validate()?;
        if !result.domain.contains_region(result.region) {
            return Err(Error::InvalidParameter(
                "work result outside declared domain",
            ));
        }

        let domain = result.domain;
        let region = result.region;
        let tile = schedule(fragment, result)?;
        let bytes = tile.slots.iter().map(Storage::bytes).sum();
        if bytes > budget {
            return Err(Error::WorkingAllowance(
                "evaluation tile exceeds scratch budget",
            ));
        }
        Ok(Self {
            domain,
            region,
            tiles: vec![tile],
            peak_scratch_bytes: bytes,
            scratch_budget: budget,
        })
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn new(root: impl Into<value::Value>, region: Rect, budget: Budget) -> Result<Self, Error> {
        Self::with_policy(root, region, budget, Policy::Materialized)
    }

    #[cfg(test)]
    pub fn with_policy(
        root: impl Into<value::Value>,
        region: Rect,
        budget: Budget,
        policy: Policy,
    ) -> Result<Self, Error> {
        Self::with_materializations(root.into(), region, budget, policy, None, &[], None)
    }

    fn with_materializations(
        root: value::Value,
        region: Rect,
        budget: Budget,
        policy: Policy,
        scope: Option<&Materializations>,
        supplied: &[SuppliedImage],
        limits: Option<&wgpu::Limits>,
    ) -> Result<Self, Error> {
        region.validate()?;
        if budget.tile.contains(&0) || !root.domain.contains_region(region) {
            return Err(Error::InvalidParameter("invalid evaluation demand"));
        }

        let mut tiles = Vec::new();
        let mut peak_scratch_bytes = 0;

        for y in (region.y..region.bottom()).step_by(budget.tile[1] as usize) {
            for x in (region.x..region.right()).step_by(budget.tile[0] as usize) {
                let region = Rect {
                    x,
                    y,
                    width: budget.tile[0].min((region.right() - x) as u32),
                    height: budget.tile[1].min((region.bottom() - y) as u32),
                };

                let mut fragment =
                    Fragment::with_programs(scope.map(|scope| scope.programs().clone()));
                let result = evaluate_region(
                    &root,
                    region,
                    &mut fragment,
                    Planning {
                        policy,
                        scope,
                        supplied,
                        scratch_bytes: budget.scratch_bytes,
                        tile: budget.tile,
                        limits,
                    },
                )?;
                let tile = schedule(fragment, result)?;

                let bytes = tile.slots.iter().map(Storage::bytes).sum();
                if bytes > budget.scratch_bytes {
                    return Err(Error::WorkingAllowance(
                        "evaluation tile exceeds scratch budget",
                    ));
                }
                peak_scratch_bytes = peak_scratch_bytes.max(bytes);
                tiles.push(tile);
            }
        }

        Ok(Self {
            domain: root.domain,
            region,
            tiles,
            peak_scratch_bytes,
            scratch_budget: budget.scratch_bytes,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Planning<'a> {
    pub policy: Policy,
    pub scope: Option<&'a Materializations>,
    pub supplied: &'a [SuppliedImage],
    pub scratch_bytes: u64,
    pub tile: [u32; 2],
    pub limits: Option<&'a wgpu::Limits>,
}

impl Planning<'_> {
    fn fits(self, fragment: &Fragment, result: PlannedImage) -> Result<bool, Error> {
        let tile = schedule(fragment.clone(), result)?;
        if tile.slots.iter().map(Storage::bytes).sum::<u64>() > self.scratch_bytes {
            return Ok(false);
        }
        let baseline = wgpu::Limits::default();
        if !tile
            .fragment
            .work
            .iter()
            .all(|work| bindings_fit(work, self.limits.unwrap_or(&baseline)))
        {
            return Ok(false);
        }
        Ok(self.limits.is_none_or(|limits| {
            tile.slots.iter().all(|storage| match storage {
                Storage::Buffer(bytes) => {
                    *bytes <= limits.max_buffer_size
                        && *bytes <= limits.max_storage_buffer_binding_size
                }
                Storage::Image { layout, .. } => {
                    layout.bytes() <= limits.max_buffer_size
                        && layout.bytes() <= limits.max_storage_buffer_binding_size
                }
                Storage::Texture(size) => {
                    size.iter().all(|v| *v <= limits.max_texture_dimension_2d)
                }
                _ => true,
            })
        }))
    }
}

fn bindings_fit(work: &Work, limits: &wgpu::Limits) -> bool {
    let bindings = match work {
        Work::Compute { bindings, .. } | Work::Render { bindings, .. } => bindings,
        _ => return true,
    };
    let mut counts = [0u32; 5];
    for (_, binding) in bindings {
        let kind = match binding {
            Binding::Shared(Shared::Buffer(buffer))
                if buffer.usage().contains(wgpu::BufferUsages::UNIFORM) =>
            {
                1
            }
            Binding::Read(_)
            | Binding::Write(_)
            | Binding::Shared(Shared::Buffer(_) | Shared::Image { view: None, .. }) => 0,
            Binding::Uniform(_) => 1,
            Binding::Texture(_)
            | Binding::Shared(Shared::Texture(_) | Shared::Image { view: Some(_), .. }) => 2,
            Binding::Shared(Shared::StorageTexture(_)) => 3,
            Binding::Shared(Shared::Sampler(_)) => 4,
        };
        counts[kind] += 1;
    }
    counts
        .into_iter()
        .zip([
            limits.max_storage_buffers_per_shader_stage,
            limits.max_uniform_buffers_per_shader_stage,
            limits.max_sampled_textures_per_shader_stage,
            limits.max_storage_textures_per_shader_stage,
            limits.max_samplers_per_shader_stage,
        ])
        .all(|(used, maximum)| used <= maximum)
}

pub(super) fn evaluate_region(
    node: &value::Value,
    region: Rect,
    fragment: &mut Fragment,
    planning: Planning<'_>,
) -> Result<PlannedImage, Error> {
    let Planning { policy, scope, .. } = planning;
    if region.width.checked_mul(region.height).is_none() {
        return Err(Error::InvalidParameter(
            "evaluation raster exceeds GPU cell addressing",
        ));
    }
    let checkpoint = (fragment.resources.len(), fragment.work.len());
    if !node.domain.contains_region(region) {
        return Err(Error::InvalidParameter(
            "evaluation dependency outside declared domain",
        ));
    }

    if let Some(stored) = Stored::find(scope, planning.supplied, node, region)? {
        let output = node.output(region)?;
        if let Some(image) = stored.raster(fragment, node.domain, region, output.identity()) {
            return Ok(image);
        }
        let output = value::image_resource(fragment, node, region, planning.scope.is_some());
        stored.sample(fragment).materialize(fragment, output)?;
        return Ok(output);
    }

    let original = node.request(region)?;
    let budget = Budget {
        tile: planning.tile,
        scratch_bytes: planning.scratch_bytes,
    };
    if !unit::single_unit(node, region, budget)? {
        let output = value::image_resource(fragment, node, region, false);
        fragment.work.push(Work::Produce {
            image: original,
            output,
        });
        return Ok(output);
    }
    let calculation = match unit::calculation(node, region, budget) {
        Ok(calculation) => calculation,
        Err(Error::WorkingAllowance(_)) => {
            let output = value::image_resource(fragment, node, region, false);
            fragment.work.push(Work::Produce {
                image: original,
                output,
            });
            return Ok(output);
        }
        Err(error) => return Err(error),
    };
    let node = calculation.clone();
    let discover = planning.scope.is_some() && value::pixel_kernel(&node);

    if policy == Policy::Composed
        && let Some(graph) = value::FunctionGraph::new(&node)?
        && graph.may_fuse(planning.limits)
    {
        let sample = graph.emit(fragment, |source, demand, fragment| {
            if policy == Policy::Composed {
                sample_region(source, demand, fragment, planning)
            } else {
                evaluate_region(source, demand, fragment, planning)
                    .map(|image| Sampled::raster(fragment, image))
            }
        })?;
        if productions_are_required(&original, region, &fragment.work[checkpoint.1..], planning)? {
            let output = value::image_resource(fragment, &original, region, discover);
            sample.materialize(fragment, output)?;
            if planning.fits(fragment, output)? {
                return Ok(output);
            }
        }
        fragment.resources.truncate(checkpoint.0);
        fragment.work.truncate(checkpoint.1);
    }

    let recorded = values::record(&original, &node, region, fragment, planning)?;
    if let Some(output) = recorded {
        if productions_are_required(&original, region, &fragment.work[checkpoint.1..], planning)?
            && planning.fits(fragment, output)?
        {
            return Ok(output);
        }
        fragment.resources.truncate(checkpoint.0);
        fragment.work.truncate(checkpoint.1);
        let output = value::image_resource(fragment, &original, region, false);
        fragment.work.push(Work::Produce {
            image: original,
            output,
        });
        return Ok(output);
    }

    let lowering = super::unit::lowered(calculation, region, budget)?;
    if matches!(lowering, super::unit::Lowering::Deferred) {
        let output = value::image_resource(fragment, &original, region, discover);
        fragment.work.push(Work::Produce {
            image: original,
            output,
        });
        return Ok(output);
    }
    if let super::unit::Lowering::Source(source) = lowering {
        let output = value::image_resource(fragment, &original, region, discover);
        fragment.work.push(Work::Upload { source, output });
        return Ok(output);
    }

    if policy == Policy::Composed
        && !value::is_copy(&node)
        && value::sampler(&node)
            .is_some_and(|producer| matches!(producer.access(), Access::Callable))
    {
        let sample = sample_region(&original, region, fragment, planning)?;
        let output = value::image_resource(fragment, &original, region, discover);
        sample.materialize(fragment, output)?;
        if productions_are_required(&original, region, &fragment.work[checkpoint.1..], planning)?
            && planning.fits(fragment, output)?
        {
            return Ok(output);
        }
        fragment.resources.truncate(checkpoint.0);
        fragment.work.truncate(checkpoint.1);
    }

    let super::unit::Lowering::Gpu {
        phase,
        inputs: operands,
    } = lowering
    else {
        return Err(Error::InvalidParameter("GPU stage has no phase"));
    };
    let inputs = phase
        .reads
        .into_iter()
        .map(|read| {
            let upstream = operands
                .get(read.input)
                .ok_or(Error::InvalidParameter("read references an unknown input"))?;
            evaluate_region(upstream, read.region, fragment, planning)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let output = value::image_resource(fragment, &original, region, discover);
    let slots = phase
        .inputs
        .iter()
        .map(|binding| match binding {
            UnitValue::Read(index) => inputs
                .get(*index)
                .map(|input| input.resource)
                .ok_or(Error::InvalidParameter("unknown phase read")),
            UnitValue::Output => Ok(output.resource),
            UnitValue::State(_) => Err(Error::InvalidParameter("unit state was not deferred")),
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let result = phase.kernel.lower(fragment, &slots)?;
    if result != output {
        return Err(Error::InvalidParameter(
            "GPU phase changed its result layout",
        ));
    }
    if !productions_are_required(&original, region, &fragment.work[checkpoint.1..], planning)?
        || !planning.fits(fragment, output)?
    {
        fragment.resources.truncate(checkpoint.0);
        fragment.work.truncate(checkpoint.1);
        let output = value::image_resource(fragment, &original, region, discover);
        fragment.work.push(Work::Produce {
            image: original,
            output,
        });
        return Ok(output);
    }
    Ok(output)
}

pub(super) fn sample_region(
    node: &value::Value,
    region: Rect,
    fragment: &mut Fragment,
    planning: Planning<'_>,
) -> Result<Sampled, Error> {
    let Planning { policy, scope, .. } = planning;
    if !node.domain.contains_region(region) {
        return Err(Error::InvalidParameter(
            "sample demand outside declared domain",
        ));
    }
    if let Some(stored) = Stored::find(scope, planning.supplied, node, region)? {
        return Ok(stored.sample(fragment));
    }
    let budget = Budget {
        tile: planning.tile,
        scratch_bytes: planning.scratch_bytes,
    };
    if !unit::single_unit(node, region, budget)? {
        let input = evaluate_region(node, region, fragment, planning)?;
        return Ok(Sampled::raster(fragment, input));
    }
    let original = node;
    let calculation = match unit::calculation(node, region, budget) {
        Ok(calculation) => calculation,
        Err(Error::WorkingAllowance(_)) => {
            let input = evaluate_region(original, region, fragment, planning)?;
            return Ok(Sampled::raster(fragment, input));
        }
        Err(error) => return Err(error),
    };
    if let Some(alias) = calculation.value()
        && !alias.same_capture(original)
    {
        return sample_region(alias, region, fragment, planning);
    }
    let node = &calculation;
    if policy == Policy::Composed
        && let Some(graph) = value::FunctionGraph::new(node)?
        && graph.may_fuse(planning.limits)
    {
        return graph.emit(fragment, |source, demand, fragment| {
            if policy == Policy::Composed {
                sample_region(source, demand, fragment, planning)
            } else {
                evaluate_region(source, demand, fragment, planning)
                    .map(|image| Sampled::raster(fragment, image))
            }
        });
    }
    if let Some(producer) = value::sampler(node) {
        let inputs = producer
            .reads(region)?
            .into_iter()
            .map(|read| {
                let upstream = node
                    .input_value(read.input)
                    .ok_or(Error::InvalidParameter("read references an unknown input"))?;
                let demand = read.region;
                Ok(match producer.read_access(read.input) {
                    Access::Raster => {
                        SampleInput::Raster(evaluate_region(upstream, demand, fragment, planning)?)
                    }
                    Access::Callable if policy == Policy::Composed => {
                        SampleInput::Callable(sample_region(upstream, demand, fragment, planning)?)
                    }
                    Access::Callable => {
                        let image = evaluate_region(upstream, demand, fragment, planning)?;
                        SampleInput::Callable(Sampled::raster(fragment, image))
                    }
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        producer.emit(fragment, inputs, node.domain(), region)
    } else {
        let input = evaluate_region(original, region, fragment, planning)?;
        Ok(Sampled::raster(fragment, input))
    }
}

pub(super) fn schedule(mut fragment: Fragment, mut result: PlannedImage) -> Result<Tile, Error> {
    // These dependencies refer to immutable calculations, not this command buffer's temporaries.
    // Fulfil them before encoding, so no unit waits for an unsubmitted parent encoder.
    fragment
        .work
        .sort_by_key(|work| !matches!(work, Work::Produce { .. }));
    if matches!(
        fragment.resources[result.resource.0],
        Storage::ResidentBuffer(_)
            | Storage::Image {
                pixels: Some(_),
                ..
            }
    ) {
        let output = fragment.image(result.domain, result.region);
        Sampled::raster(&mut fragment, result).materialize(&mut fragment, output)?;
        result = output;
    }
    if let Storage::Image {
        layout,
        pixels: None,
        ..
    } = &fragment.resources[result.resource.0]
    {
        fragment.resources[result.resource.0] = Storage::Buffer(layout.bytes());
    }
    let end = fragment.work.len();
    let mut initialized = fragment
        .resources
        .iter()
        .map(Storage::starts_initialized)
        .collect::<Vec<_>>();
    for work in &fragment.work {
        let bindings = match work {
            Work::Produce { output, .. } => {
                initialized[output.resource.0] = true;
                continue;
            }
            Work::Upload { output, .. } => {
                initialized[output.resource.0] = true;
                continue;
            }
            Work::Copy { source, target, .. } => {
                if !initialized[source.0] {
                    return Err(Error::InvalidParameter(
                        "fragment copies an uninitialized resource",
                    ));
                }
                if fragment.resources[target.0].read_only() {
                    return Err(Error::InvalidParameter(
                        "fragment copies into a read-only resource",
                    ));
                }
                initialized[target.0] = true;
                continue;
            }
            Work::Compute { bindings, .. } | Work::Render { bindings, .. } => bindings,
        };
        for (_, binding) in bindings {
            if let Binding::Read(r) | Binding::Texture(r) | Binding::Uniform(r) = binding
                && !initialized[r.0]
            {
                return Err(Error::InvalidParameter(
                    "fragment reads an uninitialized resource",
                ));
            }
        }
        for (_, binding) in bindings {
            if let Binding::Write(r) = binding {
                if fragment.resources[r.0].read_only() {
                    return Err(Error::InvalidParameter(
                        "fragment writes a read-only resident input",
                    ));
                }
                initialized[r.0] = true;
            }
        }
        if let Work::Render { target, .. } = work {
            initialized[target.0] = true;
        }
    }
    if !initialized[result.resource.0] {
        return Err(Error::InvalidParameter(
            "evaluation result is not initialized",
        ));
    }

    let mut first = fragment
        .resources
        .iter()
        .map(|storage| {
            if storage.starts_initialized() {
                0
            } else {
                usize::MAX
            }
        })
        .collect::<Vec<_>>();
    let mut last = vec![0; fragment.resources.len()];
    for (index, work) in fragment.work.iter().enumerate() {
        for Resource(id) in work.resources() {
            first[id] = first[id].min(index);
            last[id] = last[id].max(index);
        }
    }
    first[result.resource.0] = first[result.resource.0].min(end);
    last[result.resource.0] = end;

    // Queue writes precede submission of this command buffer. Uploaded source data and
    // immutable constants cannot alias storage merely because encoded uses do not overlap.
    for work in &fragment.work {
        if let Work::Upload { output, .. } = work {
            first[output.resource.0] = 0;
            last[output.resource.0] = end;
        }
    }
    for (id, storage) in fragment.resources.iter().enumerate() {
        if matches!(storage, Storage::Uniform(_) | Storage::Data(_)) {
            first[id] = 0;
            last[id] = end;
        }
    }

    let mut order = (0..first.len()).collect::<Vec<_>>();
    order.sort_by_key(|&i| first[i]);
    let mut slots: Vec<Storage> = Vec::new();
    let mut occupied_until: Vec<usize> = Vec::new();
    let mut assignment = vec![0; first.len()];
    for id in order {
        if first[id] == usize::MAX {
            return Err(Error::InvalidParameter("unused fragment resource"));
        }
        let storage = &fragment.resources[id];
        let available = slots
            .iter()
            .enumerate()
            .filter(|(s, prior)| occupied_until[*s] < first[id] && prior.can_hold(storage))
            .min_by_key(|(_, prior)| prior.bytes())
            .map(|(s, _)| s);
        let slot = available.unwrap_or_else(|| {
            slots.push(storage.clone());
            occupied_until.push(0);
            slots.len() - 1
        });
        assignment[id] = slot;
        occupied_until[slot] = last[id];
    }

    Ok(Tile {
        fragment,
        result,
        slots,
        assignment,
        last_use: last,
    })
}
