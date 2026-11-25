//! One owner for entry order and candidate lookup. Buckets name entries, not images.
use super::{Entry, RetainedKey};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct EntryId(u64);

#[derive(Default)]
pub(super) struct Entries {
    ordered: BTreeMap<EntryId, Entry>,
    buckets: HashMap<u64, BTreeSet<EntryId>>,
    next: u64,
}

impl Entries {
    pub fn ordered(&self) -> impl DoubleEndedIterator<Item = (EntryId, &Entry)> {
        self.ordered.iter().map(|(&id, entry)| (id, entry))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.ordered.values_mut()
    }

    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    pub fn last_mut(&mut self) -> Option<&mut Entry> {
        self.ordered.last_entry().map(|entry| entry.into_mut())
    }

    pub fn id_for(&self, token: &Arc<()>) -> Option<EntryId> {
        self.ordered()
            .find_map(|(id, entry)| Arc::ptr_eq(&entry.token, token).then_some(id))
    }

    pub fn get_mut(&mut self, id: EntryId) -> Option<&mut Entry> {
        self.ordered.get_mut(&id)
    }

    pub fn clear(&mut self) {
        self.ordered.clear();
        self.buckets.clear();
    }

    pub fn candidates(
        &self,
        key: &RetainedKey,
    ) -> impl DoubleEndedIterator<Item = (EntryId, &Entry)> {
        self.buckets
            .get(&key.fingerprint())
            .into_iter()
            .flatten()
            .map(|&id| (id, &self.ordered[&id]))
    }

    pub fn matching<'a>(
        &'a self,
        key: &'a RetainedKey,
    ) -> impl DoubleEndedIterator<Item = (EntryId, &'a Entry)> {
        self.candidates(key).filter(|(_, entry)| entry.key == *key)
    }

    pub fn remove(&mut self, id: EntryId) -> Option<Entry> {
        let entry = self.ordered.remove(&id)?;
        let fingerprint = entry.key.fingerprint();
        let bucket = self.buckets.get_mut(&fingerprint).unwrap();
        bucket.remove(&id);
        if bucket.is_empty() {
            self.buckets.remove(&fingerprint);
        }
        Some(entry)
    }

    pub fn push_back(&mut self, entry: Entry) -> EntryId {
        let id = EntryId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .expect("materialization order exhausted");
        self.buckets
            .entry(entry.key.fingerprint())
            .or_default()
            .insert(id);
        self.ordered.insert(id, entry);
        id
    }

    pub fn promote(&mut self, ids: impl IntoIterator<Item = EntryId>) {
        for id in ids {
            let entry = self.remove(id).unwrap();
            self.push_back(entry);
        }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Entry) -> bool) {
        let removed: Vec<_> = self
            .ordered()
            .filter_map(|(id, entry)| (!keep(entry)).then_some(id))
            .collect();
        for id in removed {
            self.remove(id);
        }
    }
}
