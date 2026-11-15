//! Explicit source projections and shared rectangular demands use ordinary image preparation.
use super::*;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

struct Node {
    value: Value,
    regions: Vec<CellRegion>,
    parents: HashSet<(usize, usize)>,
    children: HashSet<usize>,
    source_work: bool,
}

struct DependencyDemand {
    value: Value,
    region: CellRegion,
    source_projection: bool,
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
    let mut pending = vec![(root.clone(), region, None)];
    // Incomplete declarations or bounded discovery fall back to ordinary evaluation.
    let mut remaining = value::MAX_UNIT_VALUES;
    while let Some((value, region, parent)) = pending.pop() {
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
                    parents: HashSet::new(),
                    children: HashSet::new(),
                    source_work: false,
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
        if nodes[index].regions.contains(&region) {
            continue;
        }
        nodes[index].regions.push(region);
        if let Some(stored) = Stored::find(Some(scope), supplied, &value, region).ok()? {
            for image in stored.images() {
                nodes[index].source_work |= image.admitted();
                if !held.iter().any(|prior| Arc::ptr_eq(prior, image)) {
                    held.push(image.clone());
                }
            }
            continue;
        }
        nodes[index].source_work |= value.operation().source().is_some();
        for read in value.operation().reads(region).ok()? {
            let input = value.inputs().get(read.input)?;
            if !input.domain().contains_region(read.region) {
                return None;
            }
            pending.push((input.clone(), read.region, Some((index, read.input))));
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
        // Stream a missing projection before its consumers; completed projections stay inputs.
        let spatial = node.source_work
            && !node.parents.is_empty()
            && node.value.inputs().iter().any(|input| {
                input.domain().meaning == node.value.domain().meaning
                    && input.domain().grid != node.value.domain().grid
            });
        if node.parents.len() > 1 || spatial {
            let region = node
                .regions
                .first()
                .copied()
                .and_then(|first| node.regions.iter().copied().skip(1).try_fold(first, joined));
            if let Some(region) = region {
                common.push(DependencyDemand {
                    value: node.value.clone(),
                    region,
                    source_projection: spatial,
                });
            }
        }
        let parents = node
            .parents
            .iter()
            .map(|(parent, _)| *parent)
            .collect::<HashSet<_>>();
        let source_work = node.source_work;
        for parent in parents {
            nodes[parent].source_work |= source_work;
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
    pub(super) fn prepare_dependencies(
        &self,
        evaluation: &Evaluation,
    ) -> Result<Option<Preparation>, Error> {
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
        let mut held = 0u64;
        let budget = evaluation.job.budget();
        for DependencyDemand {
            value: image,
            region,
            source_projection,
        } in demands
        {
            let bytes = u64::from(region.width) * u64::from(region.height) * 16;
            let Some(available) = budget
                .scratch_bytes
                .checked_sub(held)
                .and_then(|remaining| remaining.checked_sub(bytes))
                .filter(|bytes| *bytes > 0)
            else {
                continue;
            };
            let own = Budget {
                scratch_bytes: available,
                ..budget
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
            if matches!(work.work, PreparedWork::Units) && !source_projection {
                continue;
            }
            let Some(next) = held
                .checked_add(bytes)
                .and_then(|held| held.checked_add(work.scratch_bytes()))
            else {
                continue;
            };
            if next > budget.scratch_bytes {
                continue;
            }
            let pixels = match &work.work {
                PreparedWork::Cached(image) => image.clone().try_into()?,
                _ => BufferImage::recorded(&image, region, output_buffer(self, region))?,
            };
            supplied.push(SuppliedImage::from_buffer(image, region, pixels.clone())?);
            work.destination = Some(pixels);
            dependencies.push(work);
            held = next;
        }
        if dependencies.is_empty() {
            return ordinary();
        }
        let mut request = evaluation.clone();
        request.job = request
            .job
            .with_budget(Budget {
                scratch_bytes: budget.scratch_bytes.saturating_sub(held).max(1),
                ..budget
            })
            .with_supplied(supplied.into());
        let Ok(mut result) = self.prepare_base(request) else {
            return ordinary();
        };
        if matches!(result.work, PreparedWork::Cached(_)) {
            return Ok(Some(result));
        }
        if matches!(result.work, PreparedWork::Units)
            || held
                .checked_add(result.scratch_bytes())
                .is_none_or(|bytes| bytes > budget.scratch_bytes)
        {
            return ordinary();
        }
        result.dependencies = dependencies;
        result.captured_inputs = inputs;
        Ok(Some(result))
    }
}
