use std::collections::VecDeque;

pub(super) struct Cache<K, V> {
    pub entries: VecDeque<(K, V)>,
    budget: usize,
    bytes: usize,
    weight: fn(&V) -> usize,
    protected: Vec<K>,
}
impl<K: PartialEq, V> Cache<K, V> {
    pub fn new(budget: usize, weight: fn(&V) -> usize) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            bytes: 0,
            weight,
            protected: Vec::new(),
        }
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
        self.bytes
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
            let (_, previous) = self.entries.remove(index).unwrap();
            self.bytes -= (self.weight)(&previous);
        }
        self.bytes += (self.weight)(&value);
        self.entries.push_back((key, value));
        self.trim();
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let bytes = &mut self.bytes;
        let weight = self.weight;
        self.entries.retain(|(key, value)| {
            if keep(key, value) {
                true
            } else {
                *bytes -= weight(value);
                false
            }
        });
    }
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.trim();
    }
    pub fn protect(&mut self, keys: Vec<K>) {
        self.protected = keys;
        self.trim();
    }
    fn trim(&mut self) {
        while self.bytes > self.budget {
            let Some(index) = self
                .entries
                .iter()
                .position(|(key, _)| !self.protected.contains(key))
            else {
                break;
            };
            let (_, value) = self.entries.remove(index).unwrap();
            self.bytes -= (self.weight)(&value);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::Cache;
    #[test]
    fn visible_working_set_never_thrashes_even_when_larger_than_optional_budget() {
        let mut cache = Cache::new(16, |_| 1);
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
        let mut cache = Cache::new(5, |value: &usize| *value);
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
        let mut cache = Cache::new(8, |v: &usize| *v);
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
        let mut cache = Cache::new(8, |v: &Vec<u8>| v.len());
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
