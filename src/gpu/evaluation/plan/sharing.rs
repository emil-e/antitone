//! Share a declared rectangle when it already covers all reads of the same value.
use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

struct Demand {
    value: Value,
    regions: Vec<Rect>,
    consumers: HashSet<(usize, usize)>,
}

fn containing_reads(root: &Value, region: Rect) -> Option<Vec<(Value, Rect)>> {
    let mut by_value = HashMap::new();
    let mut demands: Vec<Demand> = Vec::new();
    let mut pending = vec![(root.clone(), region, None)];
    let mut remaining = value::MAX_UNIT_VALUES;
    while let Some((value, region, consumer)) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        let index = *by_value.entry(value.pointer()).or_insert_with(|| {
            let index = demands.len();
            demands.push(Demand {
                value: value.clone(),
                regions: Vec::new(),
                consumers: HashSet::new(),
            });
            index
        });
        let demand = &mut demands[index];
        demand.consumers.extend(consumer);
        if demand.regions.contains(&region) {
            continue;
        }
        demand.regions.push(region);
        for read in value.operation().reads(region).ok()? {
            let input = value.inputs().get(read.input)?;
            if !input.domain().contains_region(read.region) {
                return None;
            }
            pending.push((input.clone(), read.region, Some((index, read.input))));
        }
    }
    let mut dependencies = vec![0usize; demands.len()];
    for demand in &demands {
        for &(parent, _) in &demand.consumers {
            dependencies[parent] += 1;
        }
    }
    let mut ready: VecDeque<_> = dependencies
        .iter()
        .enumerate()
        .filter_map(|(index, &count)| (count == 0).then_some(index))
        .collect();
    let mut shared = Vec::new();
    while let Some(index) = ready.pop_front() {
        let demand = &demands[index];
        let mut consumers: Vec<_> = demand.consumers.iter().copied().collect();
        consumers.sort_unstable();
        for &(parent, _) in &consumers {
            dependencies[parent] -= 1;
            if dependencies[parent] == 0 {
                ready.push_back(parent);
            }
        }
        if consumers.len() < 2
            || demand.value.source().is_some()
            || demand.value.shape() != value::Shape::Pixel
        {
            continue;
        }
        // The containing rectangle must itself be a declared read, not the bounding
        // box of disjoint or partially overlapping reads.
        if let Some(region) = demand.regions.iter().copied().find(|region| {
            demand
                .regions
                .iter()
                .all(|read| region.contains_region(*read))
        }) {
            shared.push((demand.value.clone(), region));
        }
    }
    Some(shared)
}

pub(super) struct Reads {
    pub(super) fragment: Fragment,
    images: Vec<(Value, PlannedImage)>,
}

impl Reads {
    pub(super) fn plan(
        root: &Value,
        region: Rect,
        planning: Planning<'_>,
    ) -> Result<Option<Self>, Error> {
        let budget = Tiling {
            tile: planning.tile,
        };
        if planning.policy != Policy::Composed || !unit::single_unit(root, region, budget)? {
            return Ok(None);
        }
        let Some(candidates) = containing_reads(root, region) else {
            return Ok(None);
        };
        let mut reads = Self {
            fragment: planning.fragment(),
            images: Vec::new(),
        };
        for (value, region) in candidates {
            if !unit::single_unit(&value, region, budget)? {
                continue;
            }
            let previous = Planning {
                shared: Some(&reads),
                ..planning
            };
            let mut fragment = reads.fragment.clone();
            let output = evaluate_region(&value, region, &mut fragment, previous)?;
            let new_work = &fragment.work[reads.fragment.work.len()..];
            if new_work.iter().any(|work| matches!(work, Work::Produce { image, output } if image.same_capture(&value) && output.region == region)) {
                continue;
            }
            reads.fragment = fragment;
            reads.images.push((value, output));
        }
        Ok((!reads.images.is_empty()).then_some(reads))
    }

    pub(super) fn get(&self, node: &Value, region: Rect) -> Option<PlannedImage> {
        self.images.iter().find_map(|(candidate, image)| {
            (candidate.same_capture(node) && image.region.contains_region(region)).then_some(*image)
        })
    }

    pub(super) fn consumed_by(&self, fragment: &Fragment) -> bool {
        self.images.iter().all(|(_, image)| {
            fragment.work.iter().any(|work| match work {
                Work::Copy { source, .. } => *source == image.resource,
                Work::Compute { bindings, .. } | Work::Render { bindings, .. } => bindings
                    .iter()
                    .any(|(_, binding)| matches!(binding, Binding::Read(resource) if *resource == image.resource)),
                _ => false,
            })
        })
    }

    pub(super) fn arguments(&self, mut fragment: Fragment) -> Fragment {
        // An isolated operand can reference completed prefix results, but must not
        // see the enclosing unit's uninitialized phase state. Keep the prefix's
        // resource numbering and map these arguments back when appending the operand.
        fragment
            .resources
            .extend(
                self.fragment
                    .resources
                    .iter()
                    .enumerate()
                    .map(|(slot, storage)| Storage::Input {
                        slot,
                        bytes: storage.bytes(),
                        writable: false,
                    }),
            );
        fragment
    }
}
