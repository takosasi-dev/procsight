//! グラフ用の直近 N 回分の入れ物(FR-24・FR-37)。履歴はメモリにだけ持つ(非スコープ: 保存)。

use std::collections::VecDeque;

pub const GRAPH_POINTS: usize = 60;

#[derive(Clone, Debug)]
pub struct Ring<T> {
    cap: usize,
    items: VecDeque<T>,
}

impl<T> Default for Ring<T> {
    fn default() -> Self {
        Ring::new(GRAPH_POINTS)
    }
}

impl<T> Ring<T> {
    pub fn new(cap: usize) -> Self {
        Ring { cap: cap.max(1), items: VecDeque::with_capacity(cap.max(1)) }
    }
    pub fn push(&mut self, v: T) {
        if self.items.len() == self.cap {
            self.items.pop_front();
        }
        self.items.push_back(v);
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.cap
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.items.iter()
    }
    pub fn last(&self) -> Option<&T> {
        self.items.back()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_last_sixty() {
        let mut r = Ring::default();
        for i in 0..100 {
            r.push(i);
        }
        assert_eq!(r.len(), 60);
        assert_eq!(r.iter().next(), Some(&40));
        assert_eq!(r.last(), Some(&99));
        r.clear();
        assert!(r.is_empty());
    }
}
