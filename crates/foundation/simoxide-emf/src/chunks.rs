//! A vector whose older part can be frozen into shared chunks: a clone copies one pointer per
//! frozen chunk plus the unfrozen tail, and a write into the frozen part copies only the chunk
//! it touches when other clones share it (copy on write). The graph keeps its objects in one:
//! a loader writes into the tail (plain vector speed), a host freezes the graph once it is
//! loaded (`Graph::freeze`), and copies of a run's model graph (forks, model changes, scratch
//! copies) then cost little however many objects it holds (`DESIGN-SPD.md` 5.9).

use std::ops::{Index, IndexMut};
use std::sync::Arc;

/// Elements per frozen chunk.
const CHUNK: usize = 256;

#[derive(Debug)]
pub(crate) struct ChunkVec<T> {
    /// The frozen prefix: `frozen` elements, all chunks full except possibly the last.
    chunks: Vec<Arc<Vec<T>>>,
    frozen: usize,
    tail: Vec<T>,
}

impl<T> Default for ChunkVec<T> {
    fn default() -> Self {
        ChunkVec {
            chunks: Vec::new(),
            frozen: 0,
            tail: Vec::new(),
        }
    }
}

impl<T: Clone> Clone for ChunkVec<T> {
    fn clone(&self) -> Self {
        ChunkVec {
            chunks: self.chunks.clone(),
            frozen: self.frozen,
            tail: self.tail.clone(),
        }
    }
}

impl<T: Clone> ChunkVec<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.frozen + self.tail.len()
    }

    #[inline]
    pub fn push(&mut self, x: T) {
        self.tail.push(x);
    }

    pub fn reserve(&mut self, n: usize) {
        self.tail.reserve(n);
    }

    /// Moves the tail into shared chunks.
    pub fn freeze(&mut self) {
        if self.tail.is_empty() {
            return;
        }
        let mut rest = std::mem::take(&mut self.tail).into_iter();
        // fill the last chunk first
        if !self.frozen.is_multiple_of(CHUNK) {
            let full = (self.chunks.len() - 1) * CHUNK;
            let last = Arc::make_mut(self.chunks.last_mut().expect("chunk"));
            let n = CHUNK - self.frozen % CHUNK;
            last.extend(rest.by_ref().take(n));
            self.frozen = full + last.len();
        }
        loop {
            let c: Vec<T> = rest.by_ref().take(CHUNK).collect();
            if c.is_empty() {
                break;
            }
            self.frozen += c.len();
            self.chunks.push(Arc::new(c));
        }
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> + DoubleEndedIterator + '_ {
        Iter {
            v: self,
            front: 0,
            back: self.len(),
        }
    }

    /// The elements from `start` on.
    pub fn iter_from(&self, start: usize) -> impl Iterator<Item = &T> + '_ {
        (start..self.len()).map(move |i| &self[i])
    }
}

struct Iter<'a, T> {
    v: &'a ChunkVec<T>,
    front: usize,
    back: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;
    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.front == self.back {
            return None;
        }
        let i = self.front;
        self.front += 1;
        Some(&self.v[i])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.back - self.front;
        (n, Some(n))
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

impl<'a, T> DoubleEndedIterator for Iter<'a, T> {
    fn next_back(&mut self) -> Option<&'a T> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        Some(&self.v[self.back])
    }
}

impl<T> Index<usize> for ChunkVec<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        if i >= self.frozen {
            &self.tail[i - self.frozen]
        } else {
            &self.chunks[i / CHUNK][i % CHUNK]
        }
    }
}

impl<T: Clone> IndexMut<usize> for ChunkVec<T> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut T {
        if i >= self.frozen {
            &mut self.tail[i - self.frozen]
        } else {
            &mut Arc::make_mut(&mut self.chunks[i / CHUNK])[i % CHUNK]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_on_write() {
        let mut a = ChunkVec::default();
        for i in 0..600 {
            a.push(i);
        }
        a.freeze();
        a.push(600);
        a.freeze();
        a.push(601);
        let b = a.clone();
        a[5] = 1000;
        a[300] = 2000;
        a.push(602);
        assert_eq!((a[5], b[5], a[300], b[300]), (1000, 5, 2000, 300));
        assert_eq!((a.len(), b.len()), (603, 602));
        assert_eq!(b.iter().copied().sum::<i32>(), (0..602).sum());
        assert_eq!(
            a.iter_from(600).copied().collect::<Vec<_>>(),
            [600, 601, 602]
        );
        assert_eq!(b.iter().next_back(), Some(&601));
        // an untouched chunk is shared
        assert!(Arc::ptr_eq(&a.chunks[2], &b.chunks[2]));
        assert!(!Arc::ptr_eq(&a.chunks[0], &b.chunks[0]));
        assert_eq!(a.chunks[2].len(), 89);
    }
}
