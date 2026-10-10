// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
pub struct RowIndex {
    rows: Vec<u32>,
    tree: Vec<u64>,
}

impl RowIndex {
    pub fn new(rows: Vec<u32>) -> Self {
        let mut r = RowIndex { rows, tree: Vec::new() };
        r.rebuild_tree();
        r
    }

    fn rebuild_tree(&mut self) {
        let n = self.rows.len();
        self.tree = vec![0; n + 1];
        for (i, &v) in self.rows.iter().enumerate() {
            self.tree[i + 1] += v as u64;
            let parent = (i + 1) + ((i + 1) & (i + 1).wrapping_neg());
            if parent <= n {
                let t = self.tree[i + 1];
                self.tree[parent] += t;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn get(&self, line: usize) -> u32 {
        self.rows[line]
    }

    pub fn prefix(&self, line: usize) -> u64 {
        let mut i = line.min(self.rows.len());
        let mut sum = 0;
        while i > 0 {
            sum += self.tree[i];
            i &= i - 1;
        }
        sum
    }

    pub fn total(&self) -> u64 {
        self.prefix(self.rows.len())
    }

    pub fn set(&mut self, line: usize, value: u32) {
        let old = self.rows[line];
        if old == value {
            return;
        }
        self.rows[line] = value;
        let mut i = line + 1;
        let n = self.rows.len();
        while i <= n {
            self.tree[i] = self.tree[i] + value as u64 - old as u64;
            i += i & i.wrapping_neg();
        }
    }

    pub fn line_at_row(&self, row: u64) -> usize {
        let n = self.rows.len();
        if n == 0 {
            return 0;
        }
        let mut pos = 0;
        let mut rem = row;
        let mut bit = 1usize << (usize::BITS - 1 - n.leading_zeros());
        while bit > 0 {
            let next = pos + bit;
            if next <= n && self.tree[next] <= rem {
                pos = next;
                rem -= self.tree[next];
            }
            bit >>= 1;
        }
        pos.min(n - 1)
    }

    pub fn splice(&mut self, first: usize, removed: usize, new_rows: Vec<u32>) {
        let end = (first + removed).min(self.rows.len());
        self.rows.splice(first.min(end)..end, new_rows);
        self.rebuild_tree();
    }
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
}
