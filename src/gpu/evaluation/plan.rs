//! Evaluation demand and schedule declared resource lifetimes, without operation-specific dispatch.

use super::CellRegion as Rect;
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Materialized,
    Composed,
}

#[derive(Clone, Copy)]
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
    root: Arc<ImageNode>,
    region: Rect,
    budget: Budget,
    policy: Policy,
    materializations: Option<Materializations>,
    supplied: Arc<[SuppliedImage]>,
    limits: Option<wgpu::Limits>,
}

impl Job {
    pub fn new(root: Arc<ImageNode>, region: Rect, budget: Budget) -> Result<Self, Error> {
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

    pub(super) fn image(&self) -> &Arc<ImageNode> {
        &self.root
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
            return Err(Error::InvalidParameter(
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
    pub fn new(root: Arc<ImageNode>, region: Rect, budget: Budget) -> Result<Self, Error> {
        Self::with_policy(root, region, budget, Policy::Materialized)
    }

    #[cfg(test)]
    pub fn with_policy(
        root: Arc<ImageNode>,
        region: Rect,
        budget: Budget,
        policy: Policy,
    ) -> Result<Self, Error> {
        Self::with_materializations(root, region, budget, policy, None, &[], None)
    }

    fn with_materializations(
        root: Arc<ImageNode>,
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

                let mut fragment = Fragment::default();
                let result = evaluate_region(
                    &root,
                    region,
                    &mut fragment,
                    Planning {
                        policy,
                        scope,
                        supplied,
                        scratch_bytes: budget.scratch_bytes,
                        limits,
                    },
                )?;
                let tile = schedule(fragment, result)?;

                let bytes = tile.slots.iter().map(Storage::bytes).sum();
                if bytes > budget.scratch_bytes {
                    return Err(Error::InvalidParameter(
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
    pub limits: Option<&'a wgpu::Limits>,
}

impl Planning<'_> {
    fn fits(self, fragment: &Fragment, result: PlannedImage) -> Result<bool, Error> {
        let tile = schedule(fragment.clone(), result)?;
        if tile.slots.iter().map(Storage::bytes).sum::<u64>() > self.scratch_bytes {
            return Ok(false);
        }
        Ok(self.limits.is_none_or(|limits| {
            tile.slots.iter().all(|storage| match storage {
                Storage::Buffer(bytes) | Storage::Retained { bytes, .. } => {
                    *bytes <= limits.max_buffer_size
                        && *bytes <= limits.max_storage_buffer_binding_size
                }
                Storage::Texture(size) => {
                    size.iter().all(|v| *v <= limits.max_texture_dimension_2d)
                }
                _ => true,
            })
        }))
    }
}

pub(super) fn probe(
    root: &Arc<ImageNode>,
    region: Rect,
    budget: Budget,
    scope: &Materializations,
    limits: &wgpu::Limits,
) -> Result<Plan, Error> {
    let mut fragment = Fragment::default();
    let result = evaluate_region(
        root,
        region,
        &mut fragment,
        Planning {
            policy: Policy::Composed,
            scope: Some(scope),
            supplied: &[],
            scratch_bytes: budget.scratch_bytes,
            limits: Some(limits),
        },
    )?;
    Plan::from_work(fragment, result, u64::MAX)
}

fn evaluate_region(
    node: &Arc<ImageNode>,
    region: Rect,
    fragment: &mut Fragment,
    planning: Planning<'_>,
) -> Result<PlannedImage, Error> {
    let Planning { policy, scope, .. } = planning;
    let checkpoint = (fragment.resources.len(), fragment.work.len());
    if !node.domain.contains_region(region) {
        return Err(Error::InvalidParameter(
            "evaluation dependency outside declared domain",
        ));
    }

    if let Some(stored) = Stored::find(scope, planning.supplied, node, region)? {
        if let Some(image) = stored.raster(fragment, node.domain, region) {
            return Ok(image);
        }
        let output = fragment.image(node.domain, region);
        stored.sample(fragment).materialize(fragment, output)?;
        return Ok(output);
    }

    if node
        .operation
        .sample_consumer()
        .and_then(|consumer| consumer.production())
        .is_some_and(|production| production.requires_worker())
    {
        let output = fragment.image(node.domain, region);
        fragment.work.push(Work::Produce {
            image: node.clone(),
            output,
        });
        return Ok(output);
    }

    if policy == Policy::Composed {
        if node.operation.sample_producer().is_some() {
            let sample = sample_region(node, region, fragment, planning)?;
            let output = fragment.image(node.domain, region);
            sample.materialize(fragment, output)?;
            return Ok(output);
        }
        if let Some(consumer) = node.operation.sample_consumer() {
            let upstream = node
                .input
                .as_ref()
                .ok_or(Error::InvalidParameter("sampling consumer has no input"))?;
            let demand = node
                .operation
                .input(region)?
                .ok_or(Error::InvalidParameter("sampling consumer has no demand"))?;
            let sample = sample_region(upstream, demand, fragment, planning)?;
            let output = fragment.image(node.domain, region);
            consumer.emit(fragment, sample, output)?;
            if consumer.production().is_some() && !planning.fits(fragment, output)? {
                fragment.resources.truncate(checkpoint.0);
                fragment.work.truncate(checkpoint.1);
                let output = fragment.image(node.domain, region);
                fragment.work.push(Work::Produce {
                    image: node.clone(),
                    output,
                });
                return Ok(output);
            }
            return Ok(output);
        }
    }

    if node.operation.pointwise().is_some() {
        let sample = sample_region(node, region, fragment, planning)?;
        let output = fragment.image(node.domain, region);
        sample.materialize(fragment, output)?;
        return Ok(output);
    }

    let mut input = match (&node.input, node.operation.input(region)?) {
        (Some(input), Some(demand)) => Some(evaluate_region(input, demand, fragment, planning)?),
        (None, None) => None,
        _ => {
            return Err(Error::InvalidParameter(
                "operation input contract differs from composition",
            ));
        }
    };
    let in_place = node.operation.in_place();
    let output = if in_place {
        let mut input = input.ok_or(Error::InvalidParameter(
            "in-place operation requires an image",
        ))?;
        if !input.domain.same_grid(node.domain) || input.region != region {
            return Err(Error::InvalidParameter(
                "in-place operation changes its grid",
            ));
        }
        if matches!(
            fragment.resources[input.resource.0],
            Storage::Input {
                writable: false,
                ..
            } | Storage::ResidentBuffer(_)
        ) {
            let copy = fragment.image(input.domain, input.region);
            lower_map(
                fragment,
                input,
                copy,
                vec![Function {
                    code: r#"
fn evaluation_own_input(p: vec4f, position: vec2f) -> vec4f {
    return p;
}
"#
                    .into(),
                    symbol: "evaluation_own_input",
                    bindings: vec![],
                }],
            )?;
            // The operation still receives an actual in-place pair, now with owned storage.
            input = copy;
        }
        PlannedImage {
            domain: node.domain,
            ..input
        }
    } else {
        fragment.image(node.domain, region)
    };
    if in_place {
        input = Some(PlannedImage {
            resource: output.resource,
            ..input.unwrap()
        });
    }
    node.operation.lower(fragment, input, output)?;
    Ok(output)
}

pub(super) fn sample_region(
    node: &Arc<ImageNode>,
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
    if let Some(function) = node.operation.pointwise() {
        let mut functions = vec![function];
        let mut cursor = node.clone();
        let upstream = loop {
            let upstream = cursor
                .input
                .as_ref()
                .ok_or(Error::InvalidParameter("pointwise node has no input"))?
                .clone();
            if !cursor.domain.same_grid(upstream.domain)
                || !upstream.domain.contains_region(region)
                || cursor.operation.input(region)? != Some(region)
            {
                return Err(Error::InvalidParameter(
                    "pointwise contract changes the grid",
                ));
            }
            if Stored::find(scope, planning.supplied, &upstream, region)?.is_some() {
                break upstream;
            }
            let Some(next) = upstream.operation.pointwise() else {
                break upstream;
            };
            functions.push(next);
            cursor = upstream;
        };
        functions.reverse();
        let input = if policy == Policy::Composed {
            sample_region(&upstream, region, fragment, planning)?
        } else {
            let raster = evaluate_region(&upstream, region, fragment, planning)?;
            Sampled::raster(fragment, raster)
        };
        return input.map(fragment, node.domain, region, functions);
    }
    if let Some(producer) = node.operation.sample_producer() {
        let input = match (node.input.as_ref(), node.operation.input(region)?) {
            (Some(upstream), Some(demand)) => Some(match producer.access() {
                Access::Raster => {
                    SampleInput::Raster(evaluate_region(upstream, demand, fragment, planning)?)
                }
                Access::Callable => {
                    SampleInput::Callable(sample_region(upstream, demand, fragment, planning)?)
                }
            }),
            (None, None) => None,
            _ => {
                return Err(Error::InvalidParameter(
                    "sample input differs from image definition",
                ));
            }
        };
        producer.emit(fragment, input, node.domain, region)
    } else {
        let input = evaluate_region(node, region, fragment, planning)?;
        Ok(Sampled::raster(fragment, input))
    }
}

fn lower_map(
    fragment: &mut Fragment,
    input: PlannedImage,
    output: PlannedImage,
    functions: Vec<Function>,
) -> Result<(), Error> {
    Sampled::raster(fragment, input)
        .map(fragment, output.domain, output.region, functions)?
        .materialize(fragment, output)
}

fn schedule(mut fragment: Fragment, mut result: PlannedImage) -> Result<Tile, Error> {
    if matches!(
        fragment.resources[result.resource.0],
        Storage::ResidentBuffer(_)
    ) {
        let output = fragment.image(result.domain, result.region);
        Sampled::raster(&mut fragment, result).materialize(&mut fragment, output)?;
        result = output;
    }
    let end = fragment.work.len();
    let mut initialized = fragment
        .resources
        .iter()
        .map(|r| {
            matches!(
                r,
                Storage::Uniform(_)
                    | Storage::Data(_)
                    | Storage::Input { .. }
                    | Storage::ResidentBuffer(_)
            )
        })
        .collect::<Vec<_>>();
    let mut uploads = 0;
    for work in &fragment.work {
        let bindings = match work {
            Work::Produce { output, .. } => {
                initialized[output.resource.0] = true;
                continue;
            }
            Work::Upload { output, .. } => {
                uploads += 1;
                initialized[output.resource.0] = true;
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
                if matches!(
                    fragment.resources[r.0],
                    Storage::Input {
                        writable: false,
                        ..
                    } | Storage::ResidentBuffer(_)
                ) {
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
    if uploads > 1 || !initialized[result.resource.0] {
        return Err(Error::InvalidParameter(
            "evaluation slice requires one source and an initialized result",
        ));
    }

    let mut first = vec![usize::MAX; fragment.resources.len()];
    let mut last = vec![0; fragment.resources.len()];
    for (index, work) in fragment.work.iter().enumerate() {
        for Resource(id) in work.resources() {
            first[id] = first[id].min(index);
            last[id] = last[id].max(index);
        }
    }
    first[result.resource.0] = first[result.resource.0].min(end);
    last[result.resource.0] = end;

    // CPU uploads precede submission. Immutable uniforms must not alias even when GPU uses
    // do not overlap; otherwise a later upload would overwrite an earlier dispatch's constants.
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
