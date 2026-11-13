//! Requested content can remain relevant even when a stored descendant supplies its pixels.
use super::*;
use std::collections::HashSet;

pub(in crate::gpu::evaluation) struct Relevance(Vec<Output>);

impl Relevance {
    pub(in crate::gpu::evaluation) fn new(root: &value::Value, region: CellRegion) -> Self {
        let mut pending = vec![(root, region)];
        let mut seen = HashSet::new();
        let mut outputs = Vec::new();
        while let Some((node, region)) = pending.pop() {
            if seen.len() == value::MAX_UNIT_VALUES {
                break;
            }
            if region.validate().is_err()
                || !node.domain().contains_region(region)
                || !seen.insert((
                    node.pointer(),
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                ))
            {
                continue;
            }
            if let Ok(output) = node.output(region) {
                outputs.push(output);
            }
            // Some stages describe reads only during lowering. Missing advisory metadata
            // loses a relevance hint, never changes execution's validation or admission.
            if let Ok(reads) = node.operation().reads(region) {
                for read in reads {
                    if pending.len() + seen.len() == value::MAX_UNIT_VALUES {
                        break;
                    }
                    if let Some(input) = node.inputs().get(read.input) {
                        pending.push((input, read.region));
                    }
                }
            }
        }
        Self(outputs)
    }
}

impl Materializations {
    pub(in crate::gpu::evaluation) fn requested(&self, root: &value::Value, region: CellRegion) {
        if self.0.retention.bytes() != 0 {
            self.refresh(&Relevance::new(root, region));
        }
    }

    pub(in crate::gpu::evaluation) fn refresh(&self, relevance: &Relevance) {
        let mut entries = self.0.entries.lock().unwrap();
        let mut relevant = Vec::new();
        for _ in 0..entries.len() {
            let entry = entries.pop_front().unwrap();
            let matches = entry
                .descriptor
                .downcast_ref::<ImageLayout>()
                .is_some_and(|layout| {
                    relevance.0.iter().any(|output| {
                        output.domain == layout.domain
                            && output.region.x < layout.region.right()
                            && layout.region.x < output.region.right()
                            && output.region.y < layout.region.bottom()
                            && layout.region.y < output.region.bottom()
                            && output.identity() == Some(&entry.key)
                    })
                });
            if matches {
                relevant.push(entry);
            } else {
                entries.push_back(entry);
            }
        }
        entries.extend(relevant);
    }

    pub(in crate::gpu::evaluation) fn used(&self, image: &Arc<ImageValue>) {
        self.used_many(std::iter::once(image));
    }

    pub(in crate::gpu::evaluation) fn used_many<'a>(
        &self,
        images: impl IntoIterator<Item = &'a Arc<ImageValue>>,
    ) {
        let images: HashSet<_> = images.into_iter().map(Arc::as_ptr).collect();
        if images.is_empty() {
            return;
        }
        let mut entries = self.0.entries.lock().unwrap();
        let mut used = Vec::new();
        for _ in 0..entries.len() {
            let entry = entries.pop_front().unwrap();
            let matches = match &entry.value {
                EntryState::Complete(value) => images.contains(&value.as_ptr()),
                _ => false,
            };
            if matches {
                used.push(entry);
            } else {
                entries.push_back(entry);
            }
        }
        entries.extend(used);
    }
}
