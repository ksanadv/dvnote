// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dvnote contributors
pub struct Utf16Map {
    pub units: Vec<u16>,
    offsets: Option<Vec<u32>>,
    chars: usize,
}

impl Utf16Map {
    pub fn new(text: &str) -> Self {
        let units: Vec<u16> = text.encode_utf16().collect();
        let chars = text.chars().count();
        let offsets = if units.len() == chars {
            None
        } else {
            let mut v = Vec::with_capacity(chars + 1);
            let mut at = 0u32;
            for c in text.chars() {
                v.push(at);
                at += c.len_utf16() as u32;
            }
            v.push(at);
            Some(v)
        };
        Utf16Map { units, offsets, chars }
    }

    pub fn char_count(&self) -> usize {
        self.chars
    }

    pub fn to_unit(&self, ch: usize) -> usize {
        match &self.offsets {
            None => ch.min(self.chars),
            Some(v) => v[ch.min(self.chars)] as usize,
        }
    }

    pub fn to_char(&self, unit: usize) -> usize {
        match &self.offsets {
            None => unit.min(self.chars),
            Some(v) => {
                let unit = unit.min(self.units.len()) as u32;
                match v.binary_search(&unit) {
                    Ok(i) => i,
                    Err(i) => i.saturating_sub(1),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_text_is_identity() {
        let m = Utf16Map::new("héllo 日本");
        assert!(m.offsets.is_none());
        assert_eq!(m.to_unit(3), 3);
        assert_eq!(m.to_char(7), 7);
    }

    #[test]
    fn surrogate_pairs_shift_offsets() {
        let m = Utf16Map::new("a🙂b日");
        assert_eq!(m.char_count(), 4);
        assert_eq!(m.units.len(), 5);
        assert_eq!([0, 1, 2, 3, 4].map(|c| m.to_unit(c)), [0, 1, 3, 4, 5]);
        assert_eq!([0, 1, 2, 3, 4, 5].map(|u| m.to_char(u)), [0, 1, 1, 2, 3, 4]);
    }

    #[test]
    fn out_of_range_is_clamped() {
        let m = Utf16Map::new("🙂🙂");
        assert_eq!(m.to_unit(99), 4);
        assert_eq!(m.to_char(99), 2);
        assert_eq!(Utf16Map::new("").to_unit(3), 0);
    }
}
