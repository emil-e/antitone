//! Pull demand and schedule declared resource lifetimes, without operation-specific dispatch.
use super::*;
use crate::geometry::SourceRegion as Rect;

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
    pub result: Image,
    pub slots: Vec<Storage>,
    pub assignment: Vec<usize>,
    pub last_use: Vec<usize>,
}
/// Captured, lazy traversal. Only one tile's physical plan is retained during execution.
#[cfg(feature = "editor")]
pub struct Job {
    root: Arc<Layer>,
    region: Rect,
    budget: Budget,
    policy: Policy,
}
#[cfg(feature = "editor")]
impl Job {
    pub fn new(root: Arc<Layer>, region: Rect, budget: Budget) -> Result<Self, Error> {
        region.validate()?;
        if budget.tile.contains(&0) || !root.domain.bounds.contains_region(region) {
            return Err(Error::InvalidParameter("invalid pull job demand"));
        }
        Ok(Self {
            root,
            region,
            budget,
            policy: Policy::Materialized,
        })
    }
    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }
    pub fn plans(&self) -> impl Iterator<Item = Result<Plan, Error>> + '_ {
        let area = self.region;
        (area.y..area.y + area.height)
            .step_by(self.budget.tile[1] as usize)
            .flat_map(move |y| {
                (area.x..area.x + area.width)
                    .step_by(self.budget.tile[0] as usize)
                    .map(move |x| {
                        let r = Rect {
                            x,
                            y,
                            width: self.budget.tile[0].min(area.x + area.width - x),
                            height: self.budget.tile[1].min(area.y + area.height - y),
                        };
                        match self.policy {
                            Policy::Materialized => Plan::new(self.root.clone(), r, self.budget),
                            Policy::Composed => {
                                Plan::with_policy(self.root.clone(), r, self.budget, self.policy)
                            }
                        }
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
    #[cfg(feature = "editor")]
    pub fn new(root: Arc<Layer>, region: Rect, budget: Budget) -> Result<Self, Error> {
        Self::with_policy(root, region, budget, Policy::Materialized)
    }
    pub fn with_policy(
        root: Arc<Layer>,
        region: Rect,
        budget: Budget,
        policy: Policy,
    ) -> Result<Self, Error> {
        region.validate()?;
        if budget.tile.contains(&0) || !root.domain.bounds.contains_region(region) {
            return Err(Error::InvalidParameter("invalid pull demand"));
        }
        let mut tiles = Vec::new();
        let mut peak_scratch_bytes = 0;
        for y in (region.y..region.y + region.height).step_by(budget.tile[1] as usize) {
            for x in (region.x..region.x + region.width).step_by(budget.tile[0] as usize) {
                let region = Rect {
                    x,
                    y,
                    width: budget.tile[0].min(region.x + region.width - x),
                    height: budget.tile[1].min(region.y + region.height - y),
                };
                let mut fragment = Fragment::default();
                let result = pull(&root, region, &mut fragment, policy)?;
                let tile = schedule(fragment, result)?;
                let bytes = tile.slots.iter().map(Storage::bytes).sum();
                if bytes > budget.scratch_bytes {
                    return Err(Error::InvalidParameter("pull tile exceeds scratch budget"));
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

fn pull(
    node: &Arc<Layer>,
    region: Rect,
    fragment: &mut Fragment,
    policy: Policy,
) -> Result<Image, Error> {
    if !node.domain.bounds.contains_region(region) {
        return Err(Error::InvalidParameter(
            "pull dependency outside declared domain",
        ));
    }
    if policy == Policy::Composed {
        if node.operation.sample_producer().is_some() {
            let sample = pull_sample(node, region, fragment, policy)?;
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
            let sample = pull_sample(upstream, demand, fragment, policy)?;
            let output = fragment.image(node.domain, region);
            consumer.emit(fragment, sample, output)?;
            return Ok(output);
        }
    }
    if let Some(function) = node.operation.pointwise() {
        let mut functions = vec![function];
        let mut cursor = node.clone();
        loop {
            let upstream = cursor
                .input
                .as_ref()
                .ok_or(Error::InvalidParameter("pointwise layer has no input"))?;
            if !cursor.domain.same_grid(upstream.domain)
                || cursor.operation.input(region)? != Some(region)
            {
                return Err(Error::InvalidParameter(
                    "pointwise contract changes the grid",
                ));
            }
            let Some(next) = upstream.operation.pointwise() else {
                break;
            };
            // Fixed binding ABIs may share a pass only when their bindings do not collide.
            if next.bindings.iter().any(|(slot, _)| {
                functions
                    .iter()
                    .any(|f| f.bindings.iter().any(|(s, _)| s == slot))
            }) {
                break;
            }
            functions.push(next);
            cursor = upstream.clone();
        }
        let input_node = cursor.input.as_ref().unwrap();
        let input = pull(input_node, region, fragment, policy)?;
        let output = fragment.image(node.domain, region);
        functions.reverse();
        lower_map(fragment, input, output, functions)?;
        return Ok(output);
    }
    let mut input = match (&node.input, node.operation.input(region)?) {
        (Some(input), Some(demand)) => Some(pull(input, demand, fragment, policy)?),
        (None, None) => None,
        _ => {
            return Err(Error::InvalidParameter(
                "layer input contract differs from composition",
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
            }
        ) {
            let copy = fragment.image(input.domain, input.region);
            lower_map(
                fragment,
                input,
                copy,
                vec![Function {
                    code: "fn pull_own_input(p:vec4f,position:vec2f)->vec4f{return p;}".into(),
                    symbol: "pull_own_input",
                    bindings: vec![],
                }],
            )?;
            // The operation still receives an actual in-place pair, now with owned storage.
            input = copy;
        }
        Image {
            domain: node.domain,
            ..input
        }
    } else {
        fragment.image(node.domain, region)
    };
    if in_place {
        input = Some(Image {
            resource: output.resource,
            ..input.unwrap()
        });
    }
    node.operation.lower(fragment, input, output)?;
    Ok(output)
}

fn pull_sample(
    node: &Arc<Layer>,
    region: Rect,
    fragment: &mut Fragment,
    policy: Policy,
) -> Result<Sampled, Error> {
    if !node.domain.bounds.contains_region(region) {
        return Err(Error::InvalidParameter(
            "sample demand outside declared domain",
        ));
    }
    if let Some(producer) = node.operation.sample_producer() {
        let upstream = node
            .input
            .as_ref()
            .ok_or(Error::InvalidParameter("sample producer has no input"))?;
        let demand = node
            .operation
            .input(region)?
            .ok_or(Error::InvalidParameter("sample producer has no demand"))?;
        let input = match producer.access() {
            Access::Raster => SampleInput::Raster(pull(upstream, demand, fragment, policy)?),
            Access::Callable => {
                SampleInput::Callable(pull_sample(upstream, demand, fragment, policy)?)
            }
        };
        producer.emit(fragment, input, node.domain, region)
    } else {
        let input = pull(node, region, fragment, policy)?;
        Ok(Sampled::raster(fragment, input))
    }
}

fn lower_map(
    fragment: &mut Fragment,
    input: Image,
    output: Image,
    functions: Vec<Function>,
) -> Result<(), Error> {
    let mut code = String::new();
    let mut modules: Vec<Arc<str>> = Vec::new();
    let mut bindings = vec![
        (20, Binding::Read(input.resource)),
        (21, Binding::Write(output.resource)),
    ];
    let origin = output.domain.point([
        f64::from(output.region.x) + 0.5,
        f64::from(output.region.y) + 0.5,
    ]);
    let geometry = [
        [
            output.region.width as f32,
            output.region.height as f32,
            0.,
            0.,
        ],
        [origin[0] as f32, origin[1] as f32, 0., 0.],
        [
            output.domain.grid.axes[0][0] as f32,
            output.domain.grid.axes[0][1] as f32,
            output.domain.grid.axes[1][0] as f32,
            output.domain.grid.axes[1][1] as f32,
        ],
    ];
    bindings.push((22, fragment.uniform(&geometry)));
    let mut calls = String::new();
    for function in functions {
        if function.bindings.iter().any(|(slot, _)| *slot >= 20) {
            return Err(Error::InvalidParameter("function collides with map ABI"));
        }
        if !modules.contains(&function.code) {
            code.push_str(&function.code);
            code.push('\n');
            modules.push(function.code);
        }
        calls.push_str(&format!("pixel = {}(pixel, position);\n", function.symbol));
        bindings.extend(
            function
                .bindings
                .into_iter()
                .map(|(slot, value)| (slot, Binding::Shared(value))),
        );
    }
    code.push_str(&format!(
        r#"
@group(0) @binding(20) var<storage, read> pull_input: array<vec4f>;
@group(0) @binding(21) var<storage, read_write> pull_output: array<vec4f>;
struct PullGeometry {{ size: vec4f, mapping: vec4f, axes: vec4f }}
@group(0) @binding(22) var<uniform> pull_geometry: PullGeometry;
@compute @workgroup_size(64)
fn pull_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {{
    let index = id.x + id.y * groups.x * 64u;
    let size = vec2u(pull_geometry.size.xy);
    if index >= size.x * size.y {{ return; }}
    let xy = vec2f(f32(index % size.x), f32(index / size.x));
    let position = pull_geometry.mapping.xy + xy.x * pull_geometry.axes.xy + xy.y * pull_geometry.axes.zw;
    var pixel = pull_input[index];
    {calls}
    pull_output[index] = pixel;
}}
"#
    ));
    fragment.compute(code, "pull_main", bindings, output.cells());
    Ok(())
}

fn schedule(fragment: Fragment, result: Image) -> Result<Tile, Error> {
    let end = fragment.work.len();
    let mut initialized = fragment
        .resources
        .iter()
        .map(|r| {
            matches!(
                r,
                Storage::Uniform(_) | Storage::Data(_) | Storage::Input { .. }
            )
        })
        .collect::<Vec<_>>();
    let mut uploads = 0;
    for work in &fragment.work {
        let bindings = match work {
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
                    }
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
            "pull slice requires one source and an initialized result",
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
