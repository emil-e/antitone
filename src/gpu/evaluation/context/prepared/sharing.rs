//! Shared rectangular demands and source acquisition slices use ordinary image preparation.
use super::*;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

struct Node {
    value: Value,
    regions: Vec<CellRegion>,
    source_slices: Vec<CellRegion>,
    parents: HashSet<(usize, usize)>,
    children: HashSet<usize>,
}

struct DependencyDemand {
    value: Value,
    region: CellRegion,
    kind: DependencyKind,
}

struct Discovery {
    demands: Vec<DependencyDemand>,
    inputs: Vec<Arc<ImageValue>>,
}

fn joined(a: CellRegion, b: CellRegion) -> Option<CellRegion> {
    if a.contains_region(b) {
        return Some(a);
    }
    if b.contains_region(a) {
        return Some(b);
    }
    let vertical = a.x == b.x && a.width == b.width && a.y <= b.bottom() && b.y <= a.bottom();
    let horizontal = a.y == b.y && a.height == b.height && a.x <= b.right() && b.x <= a.right();
    if !vertical && !horizontal {
        return None;
    }
    let low = [a.x.min(b.x), a.y.min(b.y)];
    CellRegion::new(
        low,
        [
            u32::try_from(i64::from(a.right().max(b.right())) - i64::from(low[0])).ok()?,
            u32::try_from(i64::from(a.bottom().max(b.bottom())) - i64::from(low[1])).ok()?,
        ],
    )
    .validate()
    .ok()
}

fn discover(
    root: &Value,
    region: CellRegion,
    scope: &Materializations,
    supplied: &[SuppliedImage],
) -> Option<Discovery> {
    let mut nodes = Vec::<Node>::new();
    let mut held = Vec::<Arc<ImageValue>>::new();
    let mut exact = HashMap::new();
    let mut pending = vec![(root.clone(), region, None, false)];
    // Incomplete declarations or bounded discovery fall back to ordinary evaluation.
    let mut remaining = value::MAX_UNIT_VALUES;
    while let Some((value, region, parent, acquisition)) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        let index = if let Some(index) = exact.get(&value.pointer()) {
            *index
        } else {
            let mut equivalent = None;
            for (index, node) in nodes.iter().enumerate() {
                remaining = remaining.checked_sub(1)?;
                if node.value.same_definition(&value) {
                    equivalent = Some(index);
                    break;
                }
            }
            let index = equivalent.unwrap_or_else(|| {
                let index = nodes.len();
                nodes.push(Node {
                    value: value.clone(),
                    regions: Vec::new(),
                    source_slices: Vec::new(),
                    parents: HashSet::new(),
                    children: HashSet::new(),
                });
                index
            });
            exact.insert(value.pointer(), index);
            index
        };
        if let Some((parent, input)) = parent {
            // Count consuming slots, not repeated visits while expanding their regions.
            nodes[index].parents.insert((parent, input));
            nodes[parent].children.insert(index);
        }
        let new_region = !nodes[index].regions.contains(&region);
        let new_slice = acquisition && !nodes[index].source_slices.contains(&region);
        if !new_region && !new_slice {
            continue;
        }
        if new_region {
            nodes[index].regions.push(region);
        }
        if new_slice {
            nodes[index].source_slices.push(region);
        }
        if let Some(stored) = Stored::find(Some(scope), supplied, &value, region).ok()? {
            for image in stored.images() {
                if !held.iter().any(|prior| Arc::ptr_eq(prior, image)) {
                    held.push(image.clone());
                }
            }
            continue;
        }
        for read in value.operation().reads(region).ok()? {
            let input = value.inputs().get(read.input)?;
            if !input.domain().contains_region(read.region) {
                return None;
            }
            let acquisition = read.acquisition
                || acquisition && read.region == region && input.domain().same_grid(value.domain());
            pending.push((
                input.clone(),
                read.region,
                Some((index, read.input)),
                acquisition,
            ));
        }
    }
    let mut children = nodes
        .iter()
        .map(|node| node.children.len())
        .collect::<Vec<_>>();
    let mut ready = children
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<Vec<_>>();
    let mut common = Vec::new();
    while let Some(index) = ready.pop() {
        let node = &nodes[index];
        let source = !node.source_slices.is_empty()
            && !node.parents.is_empty()
            && node.value.operation().source().is_some()
            && node.value.identity().is_some();
        if source {
            // Preserve declared source slices instead of joining them into a padded
            // rectangle for each output unit. Downstream operations can still fuse.
            for region in &node.source_slices {
                if Stored::find(Some(scope), supplied, &node.value, *region)
                    .ok()?
                    .is_none()
                {
                    common.push(DependencyDemand {
                        value: node.value.clone(),
                        region: *region,
                        kind: DependencyKind::SourceSlice,
                    });
                }
            }
        } else if node.parents.len() > 1 {
            let region = node
                .regions
                .first()
                .copied()
                .and_then(|first| node.regions.iter().copied().skip(1).try_fold(first, joined));
            if let Some(region) = region {
                common.push(DependencyDemand {
                    value: node.value.clone(),
                    region,
                    kind: DependencyKind::Shared,
                });
            }
        }
        let parents = node
            .parents
            .iter()
            .map(|(parent, _)| *parent)
            .collect::<HashSet<_>>();
        for parent in parents {
            children[parent] -= 1;
            if children[parent] == 0 {
                ready.push(parent);
            }
        }
    }
    Some(Discovery {
        demands: common,
        inputs: held,
    })
}

impl Context {
    fn admit_prefetched_units(
        &self,
        evaluation: &Evaluation,
        original_budget: Tiling,
    ) -> Result<(), Error> {
        let node = evaluation.job.image();
        let budget = evaluation.job.tiling();
        for region in node
            .operation()
            .work_units(evaluation.job.region(), original_budget)?
        {
            super::super::super::unit::plan_with_limits(
                node,
                region,
                budget,
                Some(&self.device.limits()),
            )?
            .admit_for(&self.device.limits())?;
        }
        Ok(())
    }

    pub(super) fn prepare_dependencies(
        &self,
        evaluation: &Evaluation,
    ) -> Result<Option<Preparation>, Error> {
        self.prepared_layout(evaluation)?;
        let Some(Discovery { demands, inputs }) = discover(
            evaluation.job.image(),
            evaluation.job.region(),
            &self.materializations(),
            evaluation.job.supplied(),
        ) else {
            return Ok(None);
        };
        let ordinary = || -> Result<Option<Preparation>, Error> {
            let mut work = self.prepare_base(evaluation.clone())?;
            work.captured_inputs = inputs.clone();
            Ok(Some(work))
        };
        if demands.is_empty() {
            return ordinary();
        }
        let mut dependencies = Vec::new();
        let mut supplied = evaluation.job.supplied().to_vec();
        let budget = evaluation.job.tiling();
        for DependencyDemand {
            value: image,
            region,
            kind,
        } in demands
        {
            let own = Tiling {
                tile: if image.operation().source().is_some() {
                    [region.width, region.height]
                } else {
                    budget.tile
                },
            };
            let Ok(request) = self.request(
                image.clone(),
                Region {
                    bounds: region,
                    grid: image.domain().grid,
                },
                own,
            ) else {
                continue;
            };
            let mut request = request.without_retention();
            request.job = request.job.with_supplied(supplied.clone().into());
            let Ok(mut work) = self.prepare_base(request) else {
                continue;
            };
            // Unit traversal can assemble the consumer's declared read on demand.
            if matches!(work.work, PreparedWork::Units) {
                continue;
            }
            work.kind = kind;
            let pixels = match &work.work {
                PreparedWork::Cached(image) => image.clone().try_into()?,
                _ => BufferImage::recorded(&image, region, output_buffer(self, region)?)?,
            };
            supplied.push(SuppliedImage::from_buffer(image, region, pixels.clone())?);
            work.destination = Some(pixels);
            dependencies.push(work);
        }
        let source_prefetch = evaluation.job.supplied().is_empty()
            && !dependencies.is_empty()
            && dependencies
                .iter()
                .all(|dependency| dependency.kind == DependencyKind::SourceSlice);
        if dependencies.is_empty() {
            return ordinary();
        }
        let mut request = evaluation.clone();
        request.job = request.job.with_supplied(supplied.into());
        let mut result = if source_prefetch {
            self.admit_prefetched_units(&request, budget)?;
            self.products
                .requested(request.job.image(), request.job.region());
            Preparation::new(request, PreparedWork::Units)
        } else {
            let result = self.prepare_base(request)?;
            // Deferred ordinary execution discovers source slices after publication;
            // other prepared arguments require their captured bindings.
            if matches!(result.work, PreparedWork::Units) {
                return ordinary();
            }
            result
        };
        if !matches!(result.work, PreparedWork::Cached(_)) {
            result.dependencies = dependencies;
            result.captured_inputs = inputs;
        }
        Ok(Some(result))
    }
}
