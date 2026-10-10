// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
use crate::text::Text;

fn fold(c: char, case_sensitive: bool) -> char {
    if case_sensitive {
        c
    } else {
        let mut it = c.to_lowercase();
        match (it.next(), it.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    }
}

struct Matcher {
    needle: Vec<char>,
    fail: Vec<usize>,
}

impl Matcher {
    fn new(needle: impl Iterator<Item = char>) -> Self {
        let needle: Vec<char> = needle.collect();
        let mut fail = vec![0; needle.len()];
        let mut k = 0;
        for i in 1..needle.len() {
            while k > 0 && needle[i] != needle[k] {
                k = fail[k - 1];
            }
            if needle[i] == needle[k] {
                k += 1;
            }
            fail[i] = k;
        }
        Matcher { needle, fail }
    }

    fn scan(&self, chars: impl Iterator<Item = char>) -> Option<usize> {
        let m = self.needle.len();
        let mut k = 0;
        for (i, c) in chars.enumerate() {
            while k > 0 && c != self.needle[k] {
                k = self.fail[k - 1];
            }
            if c == self.needle[k] {
                k += 1;
                if k == m {
                    return Some(i + 1);
                }
            }
        }
        None
    }
}

pub fn find(text: &Text, needle: &str, from: usize, forward: bool, case_sensitive: bool) -> Option<usize> {
    let len = text.len_chars();
    let m = needle.chars().count();
    if m == 0 || m > len {
        return None;
    }
    let from = from.min(len);
    let folded = |c: char| fold(c, case_sensitive);
    if forward {
        let fwd = Matcher::new(needle.chars().map(folded));
        if let Some(k) = fwd.scan(text.chars_from(from).map(folded)) {
            return Some(from + k - m);
        }
        let end = len.min(from + m - 1);
        fwd.scan(text.chars_from(0).take(end).map(folded)).map(|k| k - m)
    } else {
        let bwd = Matcher::new(needle.chars().rev().map(folded));
        let back = |pos: usize| text.chars_before(pos).map(folded);
        let start = len.min(from + m - 1);
        if let Some(k) = bwd.scan(back(start)) {
            return Some(start - k);
        }
        bwd.scan(back(len).take(len - from)).map(|k| len - k)
    }
}

pub fn find_all(text: &Text, needle: &str, case_sensitive: bool) -> Vec<usize> {
    let m = needle.chars().count();
    let mut out = Vec::new();
    if m == 0 || m > text.len_chars() {
        return out;
    }
    let folded = |c: char| fold(c, case_sensitive);
    let matcher = Matcher::new(needle.chars().map(folded));
    let mut pos = 0;
    while let Some(k) = matcher.scan(text.chars_from(pos).map(folded)) {
        out.push(pos + k - m);
        pos += k;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Text {
        Text::from_str(s)
    }

    #[test]
    fn forward_wraps() {
        let t = r("abc abc abc");
        assert_eq!(find(&t, "abc", 0, true, true), Some(0));
        assert_eq!(find(&t, "abc", 1, true, true), Some(4));
        assert_eq!(find(&t, "abc", 9, true, true), Some(0));
        assert_eq!(find(&t, "xyz", 0, true, true), None);
        assert_eq!(find(&t, "", 0, true, true), None);
    }

    #[test]
    fn backward_wraps() {
        let t = r("abc abc abc");
        assert_eq!(find(&t, "abc", 8, false, true), Some(4));
        assert_eq!(find(&t, "abc", 4, false, true), Some(0));
        assert_eq!(find(&t, "abc", 0, false, true), Some(8));
    }

    #[test]
    fn single_match_found_from_anywhere() {
        let t = r("xx needle yy");
        for from in 0..=12 {
            assert_eq!(find(&t, "needle", from, true, true), Some(3), "fwd from {from}");
            assert_eq!(find(&t, "needle", from, false, true), Some(3), "bwd from {from}");
        }
    }

    #[test]
    fn overlapping_candidates() {
        let t = r("aaaa");
        assert_eq!(find(&t, "aa", 0, true, true), Some(0));
        assert_eq!(find(&t, "aa", 1, true, true), Some(1));
        assert_eq!(find(&t, "aa", 3, true, true), Some(0));
        assert_eq!(find(&t, "aa", 3, false, true), Some(2));
        assert_eq!(find(&t, "aa", 2, false, true), Some(1));
    }

    #[test]
    fn case_folding() {
        assert_eq!(find(&r("Hello World"), "world", 0, true, false), Some(6));
        assert_eq!(find(&r("Hello World"), "world", 0, true, true), None);
    }

    #[test]
    fn unicode_indices_are_chars() {
        assert_eq!(find(&r("héllo wörld"), "wörld", 0, true, true), Some(6));
    }

    #[test]
    fn matches_straddling_the_start_position() {
        let t = r("abcabc");
        assert_eq!(find(&t, "abc", 1, true, true), Some(3));
        assert_eq!(find(&t, "abc", 4, true, true), Some(0));
        assert_eq!(find(&r("xabcx"), "abc", 2, true, true), Some(1));
    }

    #[test]
    fn find_all_non_overlapping() {
        assert_eq!(find_all(&r("a-b-c"), "-", true), vec![1, 3]);
        assert_eq!(find_all(&r("Aa aA"), "aa", false), vec![0, 3]);
        assert_eq!(find_all(&r("aaaa"), "aa", true), vec![0, 2]);
        assert!(find_all(&r("abc"), "", true).is_empty());
    }

    #[test]
    fn crosses_rope_chunk_boundaries() {
        let big = format!("{}needle{}", "x".repeat(50_000), "y".repeat(50_000));
        let t = r(&big);
        assert_eq!(find(&t, "needle", 0, true, true), Some(50_000));
        assert_eq!(find(&t, "needle", 99_999, false, true), Some(50_000));
        assert_eq!(find_all(&t, "needle", true), vec![50_000]);
    }
}
