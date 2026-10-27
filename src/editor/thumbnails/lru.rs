use std::collections::VecDeque;

pub(super) struct Cache<K, V> {
    pub entries: VecDeque<(K, V)>,
    budget: usize,
    bytes: usize,
    weight: fn(&K, &V) -> usize,
    shared_weight: fn(&VecDeque<(K, V)>) -> usize,
    protected: Vec<K>,
}

impl<K: PartialEq, V> Cache<K, V> {
    pub fn new(budget: usize, weight: fn(&K, &V) -> usize) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            bytes: 0,
            weight,
            shared_weight: |_| 0,
            protected: Vec::new(),
        }
    }

    pub fn with_shared_weight(mut self, weight: fn(&VecDeque<(K, V)>) -> usize) -> Self {
        self.shared_weight = weight;
        self
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let index = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = self.entries.remove(index).unwrap();
        self.entries.push_back(entry);
        self.entries.back().map(|(_, value)| value)
    }

    pub fn can_retain(&self, extra: usize) -> bool {
        self.measured_bytes()
            .checked_add(extra)
            .is_some_and(|bytes| bytes <= self.budget)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.entries.iter().any(|(candidate, _)| candidate == key)
    }

    pub fn insert(&mut self, key: K, value: V) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == &key)
        {
            self.entries.remove(index);
        }
        self.entries.push_back((key, value));
        self.trim();
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        self.entries.retain(|(key, value)| keep(key, value));
        self.bytes = self.measured_bytes();
    }

    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.trim();
    }

    pub fn protect(&mut self, keys: Vec<K>) {
        self.protected = keys;
        self.trim();
    }

    pub(super) fn measured_bytes(&self) -> usize {
        self.entries.iter().fold(
            (self.shared_weight)(&self.entries),
            |total, (key, value)| total.saturating_add((self.weight)(key, value)),
        )
    }

    fn trim(&mut self) {
        // Retained images may acquire or release downstream look resources after insertion.
        self.bytes = self.measured_bytes();
        while self.bytes > self.budget {
            let Some(index) = self
                .entries
                .iter()
                .position(|(key, _)| !self.protected.contains(key))
            else {
                break;
            };
            self.entries.remove(index);
            // A shared allocation disappears only with its last retained owner.
            self.bytes = self.measured_bytes();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::Cache;

    #[test]
    fn shared_payload_is_charged_once_until_its_last_entry_is_evicted() {
        use std::rc::Rc;
        let shared = Rc::new(8usize);
        let distinct = Rc::new(8usize);
        let mut cache = Cache::new(12, |_, _: &Rc<usize>| 2).with_shared_weight(|entries| {
            let mut seen = std::collections::HashSet::new();
            entries
                .iter()
                .filter_map(|(_, value)| seen.insert(Rc::as_ptr(value)).then_some(**value))
                .sum()
        });
        cache.insert(0, shared.clone());
        cache.insert(1, shared.clone());
        assert!(cache.contains(&0) && cache.contains(&1));
        assert!(!cache.can_retain(1));
        cache.insert(2, shared.clone());
        assert!(!cache.contains(&0));
        assert!(cache.contains(&1) && cache.contains(&2));
        cache.set_budget(10);
        assert!(cache.contains(&2));
        assert_eq!(cache.measured_bytes(), 10);
        cache.set_budget(12);
        cache.insert(3, distinct);
        assert!(
            !cache.contains(&2),
            "distinct allocations with equal contents are not shared storage"
        );
        assert_eq!(cache.measured_bytes(), 10);
        cache.set_budget(0);
        assert_eq!(cache.measured_bytes(), 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn dynamic_image_resources_are_reweighed_before_admission_eviction_and_removal() {
        use std::{cell::Cell, rc::Rc};
        let first = Rc::new(Cell::new(3usize));
        let second = Rc::new(Cell::new(3usize));
        let mut cache = Cache::new(10, |_, value: &Rc<Cell<usize>>| value.get());
        cache.insert(0, first.clone());
        cache.insert(1, second.clone());
        first.set(9);
        assert!(!cache.can_retain(1));
        cache.set_budget(10);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.contains(&1));
        assert_eq!(cache.bytes, 3);
        second.set(8);
        cache.retain(|_, _| true);
        assert_eq!(cache.bytes, 8);
        cache.insert(2, Rc::new(Cell::new(3)));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 3);
    }
    #[test]
    fn visible_working_set_never_thrashes_even_when_larger_than_optional_budget() {
        let mut cache = Cache::new(16, |_, _| 1);
        cache.protect((0..17).collect());
        for i in 0..17 {
            cache.insert(i, i);
        }
        for _ in 0..80 {
            for i in 0..17 {
                assert_eq!(cache.get(&i), Some(&i));
            }
        }
        cache.set_budget(0);
        assert_eq!(cache.entries.len(), 17);
        cache.protect((17..34).collect());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn discarding_inactive_generations_updates_the_byte_accounting() {
        let mut cache = Cache::new(5, |_, value: &usize| *value);
        cache.protect(vec![0]);
        cache.insert(0, 3);
        cache.insert(1, 2);
        cache.retain(|key, _| *key != 0);
        cache.insert(2, 3);
        assert_eq!(cache.entries.len(), 2);
        cache.set_budget(3);
        assert_eq!(cache.entries.front().unwrap().0, 2);
    }

    #[test]
    fn idle_prefetch_admission_does_not_evict_or_refresh_retained_work() {
        let mut cache = Cache::new(8, |_, v: &usize| *v);
        cache.insert(0, 4);
        cache.insert(1, 4);
        assert!(!cache.can_retain(1));
        assert!(cache.contains(&0));
        assert_eq!(cache.entries.front().unwrap().0, 0);
        cache.set_budget(12);
        assert!(cache.can_retain(4));
        assert!(!cache.can_retain(usize::MAX));
    }

    #[test]
    fn byte_accounting_replacement_lru_and_pressure_eviction() {
        let mut cache = Cache::new(8, |_, v: &Vec<u8>| v.len());
        cache.insert(0, vec![0; 4]);
        cache.insert(1, vec![0; 4]);
        cache.get(&0);
        cache.insert(2, vec![0; 4]);
        assert!(cache.get(&1).is_none());
        cache.insert(0, vec![0; 1]);
        cache.insert(3, vec![0; 3]);
        assert_eq!(cache.entries.len(), 3);
        cache.set_budget(1);
        assert!(cache.entries.is_empty());
    }
}
