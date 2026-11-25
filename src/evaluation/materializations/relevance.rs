//! Requested content can remain relevant even when a stored descendant supplies its pixels.
use super::*;
use std::collections::HashSet;

pub(in crate::evaluation) struct Relevance(Vec<Output>);

impl Relevance {
    pub(in crate::evaluation) fn new(root: &value::Value, region: CellRegion) -> Self {
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
    pub(in crate::evaluation) fn requested(&self, root: &value::Value, region: CellRegion) {
        if self.0.retention.bytes() != 0 {
            self.refresh(&Relevance::new(root, region));
        }
    }

    pub(in crate::evaluation) fn refresh(&self, relevance: &Relevance) {
        let mut entries = self.0.entries.lock().unwrap();
        let mut relevant = Vec::new();
        for (id, entry) in entries.ordered() {
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
                relevant.push(id);
            }
        }
        entries.promote(relevant);
    }

    pub(in crate::evaluation) fn used(&self, image: &Arc<ImageValue>) {
        self.used_many(std::iter::once(image));
    }

    pub(in crate::evaluation) fn used_many<'a>(
        &self,
        images: impl IntoIterator<Item = &'a Arc<ImageValue>>,
    ) {
        self.use_values(images, false);
    }

    pub(in crate::evaluation) fn submitted_values<'a>(
        &self,
        images: impl IntoIterator<Item = &'a Arc<ImageValue>>,
    ) {
        self.use_values(images, true);
    }

    fn use_values<'a>(
        &self,
        images: impl IntoIterator<Item = &'a Arc<ImageValue>>,
        submitted: bool,
    ) {
        let images: HashSet<_> = images.into_iter().map(Arc::as_ptr).collect();
        if images.is_empty() {
            return;
        }
        let mut entries = self.0.entries.lock().unwrap();
        let mut used = Vec::new();
        for (id, entry) in entries.ordered() {
            let matches = match &entry.value {
                EntryState::Complete(value) => images.contains(&value.as_ptr()),
                _ => false,
            };
            if matches {
                used.push(id);
            }
        }
        let used: Vec<_> = used
            .into_iter()
            .map(|id| entries.remove(id).unwrap())
            .collect();
        for entry in used {
            if submitted
                && entry.retain
                && entry.lease.is_none()
                && let Some(image) = entry.value.image()
                && let Some(bytes) = image.bytes()
            {
                self.completed(&mut entries, entry, &image, bytes);
            } else {
                entries.push_back(entry);
            }
        }
    }
}
