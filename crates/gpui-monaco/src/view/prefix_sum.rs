//! A Fenwick tree over `u32` counts: prefix sums and "which index contains
//! this position" in `O(log n)` (Monaco uses a prefix-sum computer for the
//! same job: mapping view lines to model lines).

#[derive(Clone, Debug, Default)]
pub struct PrefixSum {
    values: Vec<u32>,
    tree: Vec<u64>,
}

impl PrefixSum {
    pub fn new(values: Vec<u32>) -> Self {
        let mut this = Self {
            values,
            tree: Vec::new(),
        };
        this.rebuild();
        this
    }

    fn rebuild(&mut self) {
        let n = self.values.len();
        self.tree = vec![0; n + 1];
        for i in 0..n {
            let j = i + 1;
            self.tree[j] += self.values[i] as u64;
            let parent = j + (j.isolate_lowest_one());
            if parent <= n {
                let v = self.tree[j];
                self.tree[parent] += v;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn value(&self, index: usize) -> u32 {
        self.values[index]
    }

    pub fn set(&mut self, index: usize, value: u32) {
        let old = self.values[index] as i64;
        self.values[index] = value;
        let delta = value as i64 - old;
        let mut j = index + 1;
        while j < self.tree.len() {
            self.tree[j] = (self.tree[j] as i64 + delta) as u64;
            j += j.isolate_lowest_one();
        }
    }

    /// Replaces `values[range]` with `new_values` (rebuilds in `O(n)`).
    pub fn splice(
        &mut self,
        range: std::ops::Range<usize>,
        new_values: impl IntoIterator<Item = u32>,
    ) {
        self.values.splice(range, new_values);
        self.rebuild();
    }

    /// Sum of `values[..index]`.
    pub fn prefix(&self, index: usize) -> u64 {
        let mut sum = 0;
        let mut j = index.min(self.values.len());
        while j > 0 {
            sum += self.tree[j];
            j -= j.isolate_lowest_one();
        }
        sum
    }

    pub fn total(&self) -> u64 {
        self.prefix(self.values.len())
    }

    /// The index whose span contains `position` (0-based within the total),
    /// and the remainder within it. Positions past the end clamp to the last
    /// non-empty index.
    pub fn find(&self, position: u64) -> (usize, u32) {
        let n = self.values.len();
        if n == 0 {
            return (0, 0);
        }
        let total = self.total();
        if total == 0 {
            return (0, 0);
        }
        let mut position = position.min(total - 1);
        let mut index = 0;
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = index + step;
            if next <= n && self.tree[next] <= position {
                index = next;
                position -= self.tree[next];
            }
            step >>= 1;
        }
        (index, position as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_and_find() {
        let mut sums = PrefixSum::new(vec![1, 0, 3, 2, 0, 1]);
        assert_eq!(sums.total(), 7);
        assert_eq!(sums.prefix(3), 4);
        assert_eq!(sums.find(0), (0, 0));
        assert_eq!(sums.find(1), (2, 0));
        assert_eq!(sums.find(3), (2, 2));
        assert_eq!(sums.find(4), (3, 0));
        assert_eq!(sums.find(6), (5, 0));
        assert_eq!(sums.find(100), (5, 0));
        sums.set(1, 2);
        assert_eq!(sums.find(1), (1, 0));
        assert_eq!(sums.total(), 9);
        sums.splice(0..2, [5]);
        assert_eq!(sums.total(), 11);
        assert_eq!(sums.find(5), (1, 0));
    }
}
