use std;
use std::cmp::{max, min, Ordering};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Interval<T: Ord>(pub T, pub T);

impl<T: Ord> Interval<T> {
    pub fn contains(&self, time: &T) -> bool {
        time >= &self.0 && time <= &self.1
    }

    pub fn contains_interval(&self, interval: &Interval<T>) -> bool {
        self.contains(&interval.0) && self.contains(&interval.1)
    }

    pub fn intersects(&self, interval: &Interval<T>) -> bool {
        self.contains(&interval.0) || self.contains(&interval.1)
    }
}

impl<'a, T: Ord + Copy> Into<IntervalSet<T>> for &'a Interval<T> {
    fn into(self) -> IntervalSet<T> {
        let mut set = IntervalSet::new();
        set.insert(self);
        set
    }
}

pub trait UniquelyIdentifiedTimeValue<T: Ord> {
    fn time(&self) -> T;
}

struct Wrapper<Time, Value> {
    time: Time,
    value: Value,
}

impl<Time: PartialEq, Value> PartialEq for Wrapper<Time, Value> {
    fn eq(&self, other: &Wrapper<Time, Value>) -> bool {
        self.time == other.time
    }
}

impl<Time: Eq, Value> Eq for Wrapper<Time, Value> {}

impl<Time: Ord, Value> PartialOrd for Wrapper<Time, Value> {
    fn partial_cmp(&self, other: &Wrapper<Time, Value>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<Time: Ord, Value> Ord for Wrapper<Time, Value> {
    fn cmp(&self, other: &Wrapper<Time, Value>) -> Ordering {
        self.time.cmp(&other.time)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct IntervalSet<Time: Ord> {
    intervals: BTreeSet<Interval<Time>>,
}

impl<Time: Ord + Copy> IntervalSet<Time> {
    pub fn new() -> IntervalSet<Time> {
        IntervalSet {
            intervals: BTreeSet::new(),
        }
    }

    pub fn insert(&mut self, interval: &Interval<Time>) {
        // Merge any intervals which require merging
        let mut lower_bound = interval.0;
        let mut upper_bound = interval.1;
        for existing_interval in self.intersecting(&interval).iter() {
            if existing_interval.contains_interval(&interval) {
                return;
            }
            lower_bound = min(lower_bound, existing_interval.0);
            upper_bound = max(upper_bound, existing_interval.1);
            self.intervals.remove(existing_interval);
        }
        self.intervals.insert(Interval(lower_bound, upper_bound));
    }

    pub fn contains(&self, interval: &Interval<Time>) -> bool {
        self.intervals.iter().any(|i| i.contains_interval(interval))
    }

    // TODO: Find overlap more efficiently than O(n)
    pub fn intersecting(&self, interval: &Interval<Time>) -> IntervalSet<Time> {
        self.intervals
            .iter()
            .filter(|existing_interval| existing_interval.intersects(&interval))
            .map(|i| i.clone())
            .collect()
    }

    pub fn missing(&self, interval: &Interval<Time>) -> IntervalSet<Time> {
        let mut missing = BTreeSet::new();

        let mut missing_lower_bound = interval.0;

        for existing_interval in self.intervals.iter() {
            if existing_interval.1 < interval.0 {
                continue;
            } else if existing_interval.0 <= missing_lower_bound
                && existing_interval.1 >= missing_lower_bound
            {
                missing_lower_bound = existing_interval.1;
            } else if existing_interval.0 >= missing_lower_bound {
                missing.insert(Interval(
                    missing_lower_bound,
                    min(interval.1, existing_interval.0),
                ));
                missing_lower_bound = existing_interval.1;
            } else if existing_interval.0 > interval.1 {
                break;
            }
        }

        if missing_lower_bound < interval.1 {
            missing.insert(Interval(missing_lower_bound, interval.1));
        }

        IntervalSet { intervals: missing }
    }

    pub fn iter(&self) -> std::collections::btree_set::Iter<Interval<Time>> {
        self.intervals.iter()
    }
}

impl<Time: Ord> std::iter::FromIterator<Interval<Time>> for IntervalSet<Time> {
    fn from_iter<It: IntoIterator<Item = Interval<Time>>>(iter: It) -> Self {
        IntervalSet {
            intervals: BTreeSet::from_iter(iter),
        }
    }
}

pub struct IntervalStore<Time: Ord, Value: UniquelyIdentifiedTimeValue<Time> + Clone> {
    intervals: IntervalSet<Time>,
    values: BTreeSet<Wrapper<Time, Value>>,
}

impl<Time: Ord + Copy, Value: UniquelyIdentifiedTimeValue<Time> + Clone>
    IntervalStore<Time, Value>
{
    pub fn new() -> IntervalStore<Time, Value> {
        IntervalStore {
            intervals: IntervalSet::new(),
            values: BTreeSet::new(),
        }
    }

    pub fn has(&self, interval: &Interval<Time>) -> bool {
        self.intervals.contains(interval)
    }

    pub fn missing(&self, interval: &Interval<Time>) -> IntervalSet<Time> {
        self.intervals.missing(interval)
    }

    pub fn get(&self, interval: &Interval<Time>) -> Option<Vec<Value>> {
        if !self.has(interval) {
            return None;
        }
        // TODO: Use range
        return Some(
            self.values
                .iter()
                .filter(|w| interval.contains(&w.time))
                .map(|w| w.value.clone())
                .collect(),
        );
    }

    pub fn insert(&mut self, interval: &Interval<Time>, values: Vec<Value>) -> Result<(), String> {
        let mut wrapped_values: BTreeSet<_> = values
            .into_iter()
            .map(|v| Wrapper {
                time: v.time(),
                value: v,
            })
            .collect();

        let overlapping_existing_intervals = self.intervals.intersecting(&interval);

        for existing_interval in overlapping_existing_intervals.iter() {
            let overlap = Interval(
                max(existing_interval.0, interval.0),
                min(existing_interval.1, interval.1),
            );
            if wrapped_values
                .iter()
                .filter(|w| overlap.contains(&w.time))
                .collect::<Vec<_>>()
                != self
                    .values
                    .iter()
                    .filter(|w| overlap.contains(&w.time))
                    .collect::<Vec<_>>()
            {
                return Err(format!("Conflicting values"));
            }
        }

        self.intervals.insert(&interval);

        self.values.append(&mut wrapped_values);

        Ok(())
    }
}

#[cfg(test)]
mod intervalset_tests {
    use super::{Interval, IntervalSet};

    #[test]
    fn contains_empty() {
        let set = IntervalSet::new();
        assert!(!set.contains(&Interval(10, 20)));
    }

    #[test]
    fn contains_part() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(10, 15));
        assert!(!set.contains(&Interval(10, 20)));
    }

    #[test]
    fn contains_exact() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(10, 20));
        assert!(set.contains(&Interval(10, 20)));
    }

    #[test]
    fn contains_more() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(5, 25));
        assert!(set.contains(&Interval(10, 20)));
    }

    #[test]
    fn missing_none() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(10, 20));
        assert_eq!(set.missing(&Interval(10, 20)), IntervalSet::new());
        assert_eq!(set.missing(&Interval(12, 15)), IntervalSet::new());
    }

    #[test]
    fn missing_lower() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(10, 20));
        assert_eq!(set.missing(&Interval(5, 10)), interval_set(Interval(5, 10)));
        assert_eq!(set.missing(&Interval(5, 15)), interval_set(Interval(5, 10)));
    }

    #[test]
    fn missing_upper() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(10, 20));
        assert_eq!(
            set.missing(&Interval(20, 25)),
            interval_set(Interval(20, 25))
        );
        assert_eq!(
            set.missing(&Interval(15, 25)),
            interval_set(Interval(20, 25))
        );
    }

    #[test]
    fn missing_middle() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(5, 10));
        set.insert(&Interval(20, 30));
        assert_eq!(
            set.missing(&Interval(12, 15)),
            interval_set(Interval(12, 15))
        );
        assert_eq!(
            set.missing(&Interval(10, 15)),
            interval_set(Interval(10, 15))
        );
        assert_eq!(
            set.missing(&Interval(15, 20)),
            interval_set(Interval(15, 20))
        );
        assert_eq!(
            set.missing(&Interval(15, 25)),
            interval_set(Interval(15, 20))
        );
    }

    #[test]
    fn missing_multi() {
        let mut set = IntervalSet::new();
        set.insert(&Interval(5, 10));
        set.insert(&Interval(20, 30));
        assert_eq!(
            set.missing(&Interval(1, 40)),
            interval_set_of(vec![Interval(1, 5), Interval(10, 20), Interval(30, 40)])
        );
    }

    fn interval_set(interval: Interval<u32>) -> IntervalSet<u32> {
        interval_set_of(vec![interval])
    }

    fn interval_set_of(intervals: Vec<Interval<u32>>) -> IntervalSet<u32> {
        IntervalSet {
            intervals: intervals.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod intervalstore_tests {
    use super::{Interval, IntervalStore, UniquelyIdentifiedTimeValue};

    #[test]
    fn get_missing() {
        let store = new();
        assert_eq!(store.get(&Interval(10, 20)), None);
    }

    #[test]
    fn get_empty_bucket() {
        let mut store = new();
        store.insert(&Interval(10, 20), vec![]).expect("Insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![]));
    }

    #[test]
    fn get_whole_bucket() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("Insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15]));
    }

    #[test]
    fn get_part_of_bucket() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("Insert");
        assert_eq!(store.get(&Interval(10, 14)), Some(vec![10, 11]));
    }

    #[test]
    fn insert() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("Insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15]));
    }

    #[test]
    fn reinsert_idempotent_whole_interval() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("First insert");
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15]));
    }

    #[test]
    fn reinsert_conflict_whole_interval() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("First insert");
        store
            .insert(&Interval(10, 20), vec![14])
            .expect_err("Second insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15]));
    }

    #[test]
    fn reinsert_missing_some_whole_interval() {
        let mut store = new();
        store
            .insert(&Interval(10, 20), vec![10, 11, 15])
            .expect("First insert");
        store
            .insert(&Interval(10, 20), vec![11])
            .expect_err("Second insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15]));
    }

    #[test]
    fn insert_adjacent_interval_no_overlapping_value() {
        let mut store = new();
        store
            .insert(&Interval(15, 20), vec![16])
            .expect("First insert");
        store
            .insert(&Interval(10, 15), vec![10, 11])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 16]));
    }

    #[test]
    fn insert_adjacent_interval_overlapping_value() {
        let mut store = new();
        store
            .insert(&Interval(15, 20), vec![15, 16])
            .expect("First insert");
        store
            .insert(&Interval(10, 15), vec![10, 11, 15])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(10, 20)), Some(vec![10, 11, 15, 16]));
    }

    #[test]
    fn insert_bottom_overlapping_interval() {
        let mut store = new();
        store
            .insert(&Interval(10, 15), vec![10, 11, 15])
            .expect("First insert");
        store
            .insert(&Interval(8, 12), vec![9, 10, 11])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(8, 15)), Some(vec![9, 10, 11, 15]));
    }

    #[test]
    fn insert_top_overlapping_interval() {
        let mut store = new();
        store
            .insert(&Interval(8, 12), vec![9, 10, 11])
            .expect("First insert");
        store
            .insert(&Interval(10, 15), vec![10, 11, 15])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(8, 15)), Some(vec![9, 10, 11, 15]));
    }

    #[test]
    fn insert_contained_interval() {
        let mut store = new();
        store
            .insert(&Interval(8, 15), vec![9, 10, 11, 15])
            .expect("First insert");
        store
            .insert(&Interval(10, 15), vec![10, 11, 15])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(8, 15)), Some(vec![9, 10, 11, 15]));
    }

    #[test]
    fn insert_disjoint_interval() {
        let mut store = new();
        store
            .insert(&Interval(8, 9), vec![9])
            .expect("First insert");
        store
            .insert(&Interval(12, 15), vec![15])
            .expect("Second insert");
        assert_eq!(store.get(&Interval(8, 9)), Some(vec![9]));
        assert_eq!(store.get(&Interval(8, 15)), None);
    }

    fn new() -> IntervalStore<u64, u32> {
        IntervalStore::new()
    }

    impl UniquelyIdentifiedTimeValue<u64> for u32 {
        fn time(&self) -> u64 {
            *self as u64
        }
    }
}
