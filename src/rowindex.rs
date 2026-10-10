// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
// Rows per line (or pixels per preview block) with prefix sums and the inverse, row -> line.
//
// Two representations, chosen by what the content looks like:
// * Sparse: every line has `base` rows except a short sorted list of exceptions. A large file starts this way (all
//   lines estimated alike) and only the lines that get laid out deviate, so it costs a few bytes instead of several per
//   line. It turns Dense once the exceptions stop being few.
// * Dense: a Fenwick tree of u32 and nothing else, 4 bytes per line; single values are read back from the tree.
//
// Sums are kept in u32 with wrapping arithmetic, which is exact as long as the true total fits. For the editor the
// total is at most the number of characters plus the number of lines, and files are limited to 2 GiB.

// below this many lines Dense is small anyway and simpler to update
const SPARSE_MIN_LINES: usize = 1 << 16;
// Sparse updates are linear in the exceptions, so their number is capped
const SPARSE_MAX_EXCEPTIONS: usize = 1 << 14;

pub enum RowIndex {
    Sparse {
        n: usize,
        base: u32,
        // (line, rows), sorted by line, rows != base
        ex: Vec<(u32, u32)>,
        // before[j]: sum of (rows - base) over ex[..j]
        before: Vec<i64>,
    },
    Dense {
        // Fenwick tree, 1-based: tree[i] covers lines (i - lowbit(i))..i
        tree: Vec<u32>,
    },
}

fn lowbit(i: usize) -> usize {
    i & i.wrapping_neg()
}

impl RowIndex {
    pub fn new(rows: Vec<u32>) -> Self {
        if rows.len() >= SPARSE_MIN_LINES && rows[0] > 0 {
            let base = rows[0];
            let ex: Vec<(u32, u32)> =
                rows.iter().enumerate().filter(|(_, &r)| r != base).map(|(i, &r)| (i as u32, r)).collect();
            if ex.len() <= SPARSE_MAX_EXCEPTIONS {
                let mut r = RowIndex::Sparse { n: rows.len(), base, ex, before: Vec::new() };
                r.refresh_before(0);
                return r;
            }
        }
        RowIndex::Dense { tree: build_tree(rows) }
    }

    // n lines of `base` rows each, without materializing them
    pub fn uniform(n: usize, base: u32) -> Self {
        if n >= SPARSE_MIN_LINES && base > 0 {
            RowIndex::Sparse { n, base, ex: Vec::new(), before: vec![0] }
        } else {
            RowIndex::Dense { tree: build_tree(vec![base; n]) }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            RowIndex::Sparse { n, .. } => *n,
            RowIndex::Dense { tree } => tree.len() - 1,
        }
    }

    pub fn get(&self, line: usize) -> u32 {
        match self {
            RowIndex::Sparse { base, ex, .. } => match ex.binary_search_by_key(&(line as u32), |e| e.0) {
                Ok(j) => ex[j].1,
                Err(_) => *base,
            },
            RowIndex::Dense { tree } => {
                // the tree node minus the nodes it covers below
                let i = line + 1;
                let mut v = tree[i];
                let (stop, mut k) = (i - lowbit(i), i - 1);
                while k > stop {
                    v = v.wrapping_sub(tree[k]);
                    k -= lowbit(k);
                }
                v
            }
        }
    }

    pub fn prefix(&self, line: usize) -> u64 {
        let line = line.min(self.len());
        match self {
            RowIndex::Sparse { base, ex, before, .. } => {
                let j = ex.partition_point(|e| (e.0 as usize) < line);
                (line as i64 * *base as i64 + before[j]) as u64
            }
            RowIndex::Dense { tree } => {
                let (mut i, mut sum) = (line, 0u32);
                while i > 0 {
                    sum = sum.wrapping_add(tree[i]);
                    i &= i - 1;
                }
                sum as u64
            }
        }
    }

    pub fn total(&self) -> u64 {
        self.prefix(self.len())
    }

    pub fn set(&mut self, line: usize, value: u32) {
        let old = self.get(line);
        if old == value {
            return;
        }
        match self {
            RowIndex::Sparse { base, ex, .. } => {
                let j = match ex.binary_search_by_key(&(line as u32), |e| e.0) {
                    Ok(j) if value == *base => {
                        ex.remove(j);
                        j
                    }
                    Ok(j) => {
                        ex[j].1 = value;
                        j
                    }
                    Err(_) if value == *base => return,
                    Err(j) => {
                        ex.insert(j, (line as u32, value));
                        j
                    }
                };
                if ex.len() > SPARSE_MAX_EXCEPTIONS {
                    self.make_dense();
                } else {
                    self.refresh_before(j);
                }
            }
            RowIndex::Dense { tree } => {
                let delta = value.wrapping_sub(old);
                let mut i = line + 1;
                while i < tree.len() {
                    tree[i] = tree[i].wrapping_add(delta);
                    i += lowbit(i);
                }
            }
        }
    }

    pub fn line_at_row(&self, row: u64) -> usize {
        let n = self.len();
        if n == 0 {
            return 0;
        }
        match self {
            RowIndex::Sparse { base, ex, before, .. } => {
                let (base, row) = (*base as i64, row.min(i64::MAX as u64) as i64);
                // first row of exception j; never decreases with j, since every line has at least zero rows
                let start = |j: usize| ex[j].0 as i64 * base + before[j];
                // j = how many exceptions start at or before `row`
                let (mut j, mut hi) = (0, ex.len());
                while j < hi {
                    let mid = (j + hi) / 2;
                    if start(mid) <= row {
                        j = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                let line = if j == 0 {
                    (row / base) as usize
                } else {
                    let (line, rows) = ex[j - 1];
                    let end = start(j - 1) + rows as i64;
                    if row < end {
                        line as usize
                    } else {
                        line as usize + 1 + ((row - end) / base) as usize
                    }
                };
                line.min(n - 1)
            }
            RowIndex::Dense { tree } => {
                let (mut pos, mut rem) = (0, row);
                let mut bit = 1usize << (usize::BITS - 1 - n.leading_zeros());
                while bit > 0 {
                    let next = pos + bit;
                    if next <= n && tree[next] as u64 <= rem {
                        pos = next;
                        rem -= tree[next] as u64;
                    }
                    bit >>= 1;
                }
                pos.min(n - 1)
            }
        }
    }

    pub fn splice(&mut self, first: usize, removed: usize, new_rows: Vec<u32>) {
        let n = self.len();
        let end = (first + removed).min(n);
        let first = first.min(end);
        match self {
            RowIndex::Sparse { n, base, ex, .. } => {
                let (base, added) = (*base, new_rows.len());
                let shift = |line: u32| (line as usize - (end - first) + added) as u32;
                let lo = ex.partition_point(|e| (e.0 as usize) < first);
                let hi = ex.partition_point(|e| (e.0 as usize) < end);
                let inserted: Vec<(u32, u32)> = new_rows
                    .iter()
                    .enumerate()
                    .filter(|(_, &r)| r != base)
                    .map(|(i, &r)| ((first + i) as u32, r))
                    .collect();
                let tail: Vec<(u32, u32)> = ex[hi..].iter().map(|&(l, r)| (shift(l), r)).collect();
                ex.truncate(lo);
                ex.extend(inserted);
                ex.extend(tail);
                *n = *n - (end - first) + added;
                if ex.len() > SPARSE_MAX_EXCEPTIONS {
                    self.make_dense();
                } else {
                    self.refresh_before(lo);
                }
            }
            RowIndex::Dense { tree } => {
                let mut rows = values(std::mem::take(tree));
                rows.splice(first..end, new_rows);
                *tree = build_tree(rows);
            }
        }
    }

    fn refresh_before(&mut self, from: usize) {
        let RowIndex::Sparse { base, ex, before, .. } = self else { return };
        before.resize(ex.len() + 1, 0);
        let base = *base as i64;
        for j in from..ex.len() {
            before[j + 1] = before[j] + ex[j].1 as i64 - base;
        }
    }

    fn make_dense(&mut self) {
        let RowIndex::Sparse { n, base, ex, .. } = self else { return };
        let mut rows = vec![*base; *n];
        for &(line, r) in ex.iter() {
            rows[line as usize] = r;
        }
        *self = RowIndex::Dense { tree: build_tree(rows) };
    }
}

// in place, linear: each node adds itself to its parent
fn build_tree(rows: Vec<u32>) -> Vec<u32> {
    let mut tree = Vec::with_capacity(rows.len() + 1);
    tree.push(0);
    tree.extend(rows);
    let n = tree.len() - 1;
    for i in 1..=n {
        let parent = i + lowbit(i);
        if parent <= n {
            tree[parent] = tree[parent].wrapping_add(tree[i]);
        }
    }
    tree
}

// the inverse of build_tree, also in place
fn values(mut tree: Vec<u32>) -> Vec<u32> {
    let n = tree.len() - 1;
    for i in (1..=n).rev() {
        let parent = i + lowbit(i);
        if parent <= n {
            tree[parent] = tree[parent].wrapping_sub(tree[i]);
        }
    }
    tree.remove(0);
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_prefix(rows: &[u32], line: usize) -> u64 {
        rows[..line].iter().map(|&v| v as u64).sum()
    }

    #[test]
    fn prefix_matches_naive() {
        let rows: Vec<u32> = (0..1000).map(|i| 1 + (i * 7 % 5)).collect();
        let r = RowIndex::new(rows.clone());
        for line in [0, 1, 2, 17, 500, 999, 1000] {
            assert_eq!(r.prefix(line), naive_prefix(&rows, line), "line {line}");
        }
        assert_eq!(r.total(), naive_prefix(&rows, 1000));
    }

    #[test]
    fn line_at_row_inverts_prefix() {
        let rows: Vec<u32> = (0..333).map(|i| 1 + (i * 13 % 4)).collect();
        let r = RowIndex::new(rows.clone());
        for line in 0..rows.len() {
            let start = r.prefix(line);
            assert_eq!(r.line_at_row(start), line);
            assert_eq!(r.line_at_row(start + rows[line] as u64 - 1), line);
        }
        assert_eq!(r.line_at_row(u64::MAX), rows.len() - 1);
    }

    #[test]
    fn set_updates_sums() {
        let mut r = RowIndex::new(vec![1; 10]);
        r.set(3, 5);
        r.set(9, 2);
        assert_eq!(r.total(), 15);
        assert_eq!(r.prefix(4), 3 + 5);
        assert_eq!(r.line_at_row(3), 3);
        assert_eq!(r.line_at_row(7), 3);
        assert_eq!(r.line_at_row(8), 4);
    }

    #[test]
    fn splice_changes_line_count() {
        let mut r = RowIndex::new(vec![1, 2, 3, 4]);
        r.splice(1, 2, vec![7]);
        assert_eq!(r.len(), 3);
        assert_eq!(r.total(), 1 + 7 + 4);
        assert_eq!(r.line_at_row(1), 1);
        assert_eq!(r.line_at_row(8), 2);
        r.splice(3, 0, vec![2, 2]);
        assert_eq!(r.len(), 5);
        assert_eq!(r.total(), 1 + 7 + 4 + 4);
    }

    #[test]
    fn single_and_empty() {
        let r = RowIndex::new(vec![3]);
        assert_eq!(r.line_at_row(0), 0);
        assert_eq!(r.line_at_row(10), 0);
        assert_eq!(RowIndex::new(vec![]).line_at_row(5), 0);
    }

    #[test]
    fn uniform_large_index_is_sparse_and_tiny() {
        let r = RowIndex::uniform(15_000_000, 1);
        assert!(matches!(r, RowIndex::Sparse { .. }));
        assert_eq!(r.total(), 15_000_000);
        assert_eq!(r.line_at_row(9_999_999), 9_999_999);
        assert_eq!(r.get(123), 1);
    }

    // xorshift, so the test needs no crates
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    fn check(r: &RowIndex, model: &[u32], rng: &mut Rng) {
        assert_eq!(r.len(), model.len());
        assert_eq!(r.total(), naive_prefix(model, model.len()));
        for _ in 0..40 {
            let line = rng.below(model.len());
            assert_eq!(r.get(line), model[line], "get {line}");
            let start = naive_prefix(model, line);
            assert_eq!(r.prefix(line), start, "prefix {line}");
            if model[line] > 0 {
                assert_eq!(r.line_at_row(start), line, "start row of {line}");
                assert_eq!(r.line_at_row(start + model[line] as u64 - 1), line, "end row of {line}");
            }
        }
    }

    // random edits against a plain Vec, starting both sparse and dense, including the switch from one to the other
    #[test]
    fn random_operations_match_a_vec() {
        for (seed, n, base, start_dense) in [(7, 70_000, 1, false), (11, 70_000, 3, false), (13, 5_000, 2, true)] {
            let mut rng = Rng(seed);
            let mut model = vec![base; n];
            let mut r = if start_dense { RowIndex::new(model.clone()) } else { RowIndex::uniform(n, base) };
            for step in 0..3_000 {
                match rng.below(10) {
                    0..=5 => {
                        let line = rng.below(model.len());
                        let v = if rng.below(3) == 0 { base } else { 1 + rng.below(6) as u32 };
                        model[line] = v;
                        r.set(line, v);
                    }
                    6..=7 => {
                        let first = rng.below(model.len());
                        let removed = rng.below(4).min(model.len() - first);
                        let new: Vec<u32> = (0..rng.below(4)).map(|_| 1 + rng.below(5) as u32).collect();
                        model.splice(first..first + removed, new.clone());
                        r.splice(first, removed, new);
                    }
                    _ => {
                        // a burst of measurements, like scrolling through a wrapped file
                        let at = rng.below(model.len());
                        for line in at..(at + 400).min(model.len()) {
                            let v = 1 + (line % 4) as u32;
                            model[line] = v;
                            r.set(line, v);
                        }
                    }
                }
                if step % 50 == 0 {
                    check(&r, &model, &mut rng);
                }
            }
            check(&r, &model, &mut rng);
        }
    }

    #[test]
    fn values_inverts_build_tree() {
        let rows: Vec<u32> = (0..1234).map(|i| (i * 31 % 17) as u32).collect();
        assert_eq!(values(build_tree(rows.clone())), rows);
    }
}
