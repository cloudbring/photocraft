//! Persistent, bitmap-indexed radix directory, implemented from the HAMT algorithm.
//! Signed coordinates are biased and packed x-first; most-significant nibbles select
//! branches, so a depth-first walk has exactly `TileCoord::Ord` order without sorting.
//! Leaves terminate early. Paths have at most 16 branches, independent of input, and
//! `Arc::make_mut` copies only shared paths, never the pixel buffers themselves.

use std::{fmt, sync::Arc};

use photocraft_geom::TileCoord;

use crate::Tile;

#[derive(Clone, Default)]
pub(crate) struct TileDir {
    root: Option<Arc<Node>>,
    len: usize,
}

#[derive(Clone)]
enum Node {
    Leaf(TileCoord, Arc<Tile>),
    Branch { bitmap: u16, children: Vec<Arc<Node>> },
}

impl fmt::Debug for TileDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

fn packed(coord: TileCoord) -> u64 {
    (u64::from((coord.tx as u32) ^ 0x8000_0000) << 32) | u64::from((coord.ty as u32) ^ 0x8000_0000)
}

fn bit(key: u64, shift: u32) -> u16 {
    1 << ((key >> shift) & 15)
}

fn rank(bitmap: u16, bit: u16) -> usize {
    (bitmap & (bit - 1)).count_ones() as usize
}

impl Node {
    fn get(&self, coord: TileCoord, key: u64, shift: u32) -> Option<&Arc<Tile>> {
        match self {
            Self::Leaf(stored, tile) => (*stored == coord).then_some(tile),
            Self::Branch { bitmap, children } => {
                let bit = bit(key, shift);
                if bitmap & bit == 0 {
                    return None;
                }
                children.get(rank(*bitmap, bit))?.get(coord, key, shift.saturating_sub(4))
            }
        }
    }

    fn get_mut(node: &mut Arc<Self>, coord: TileCoord, key: u64, shift: u32) -> Option<&mut Arc<Tile>> {
        match Arc::make_mut(node) {
            Self::Leaf(stored, tile) => (*stored == coord).then_some(tile),
            Self::Branch { bitmap, children } => {
                let bit = bit(key, shift);
                if *bitmap & bit == 0 {
                    return None;
                }
                Self::get_mut(children.get_mut(rank(*bitmap, bit))?, coord, key, shift.saturating_sub(4))
            }
        }
    }

    /// A branch's bitmap and compact children always have matching cardinality.
    /// Rank is therefore in bounds after inserting a missing child. Distinct full
    /// keys must diverge by the final nibble, bounding splitting to 16 levels.
    fn entry<'a>(node: &'a mut Arc<Self>, coord: TileCoord, key: u64, shift: u32, create: &mut impl FnMut() -> Arc<Tile>) -> (&'a mut Arc<Tile>, bool) {
        if let Self::Leaf(stored, _) = node.as_ref()
            && *stored != coord
        {
            *node = Arc::new(Self::Branch { bitmap: bit(packed(*stored), shift), children: vec![node.clone()] });
        }
        match Arc::make_mut(node) {
            Self::Leaf(_, tile) => (tile, false),
            Self::Branch { bitmap, children } => {
                let bit = bit(key, shift);
                let index = rank(*bitmap, bit);
                if *bitmap & bit == 0 {
                    children.insert(index, Arc::new(Self::Leaf(coord, create())));
                    *bitmap |= bit;
                    let (tile, _) = Self::entry(&mut children[index], coord, key, shift.saturating_sub(4), create);
                    (tile, true)
                } else {
                    Self::entry(&mut children[index], coord, key, shift.saturating_sub(4), create)
                }
            }
        }
    }

    fn remove(node: &mut Arc<Self>, coord: TileCoord, key: u64, shift: u32) -> (Option<Arc<Tile>>, bool) {
        match Arc::make_mut(node) {
            Self::Leaf(stored, tile) => {
                let matched = *stored == coord;
                (matched.then(|| tile.clone()), matched)
            }
            Self::Branch { bitmap, children } => {
                let bit = bit(key, shift);
                if *bitmap & bit == 0 {
                    return (None, false);
                }
                let index = rank(*bitmap, bit);
                let Some(child) = children.get_mut(index) else { return (None, false) };
                let (removed, empty) = Self::remove(child, coord, key, shift.saturating_sub(4));
                if empty {
                    children.remove(index);
                    *bitmap &= !bit;
                }
                let empty = children.is_empty();
                if let [child] = children.as_slice()
                    && matches!(child.as_ref(), Self::Leaf(..))
                {
                    *node = child.clone();
                }
                (removed, empty)
            }
        }
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Leaf(coord, tile), Self::Leaf(other_coord, other_tile)) => coord == other_coord && (Arc::ptr_eq(tile, other_tile) || tile == other_tile),
            (Self::Branch { bitmap, children }, Self::Branch { bitmap: other_bitmap, children: other_children }) => {
                bitmap == other_bitmap && children.iter().zip(other_children).all(|(left, right)| Arc::ptr_eq(left, right) || left == right)
            }
            _ => false,
        }
    }
}

impl PartialEq for TileDir {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && match (&self.root, &other.root) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right) || left == right,
                _ => false,
            }
    }
}

impl TileDir {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn get(&self, coord: &TileCoord) -> Option<&Arc<Tile>> {
        self.root.as_ref()?.get(*coord, packed(*coord), 60)
    }

    pub(crate) fn contains_key(&self, coord: &TileCoord) -> bool {
        self.get(coord).is_some()
    }

    pub(crate) fn get_mut(&mut self, coord: &TileCoord) -> Option<&mut Arc<Tile>> {
        self.get(coord)?;
        Node::get_mut(self.root.as_mut()?, *coord, packed(*coord), 60)
    }

    pub(crate) fn get_or_insert_with(&mut self, coord: TileCoord, mut create: impl FnMut() -> Arc<Tile>) -> &mut Arc<Tile> {
        let root = self.root.get_or_insert_with(|| {
            self.len += 1;
            Arc::new(Node::Leaf(coord, create()))
        });
        let (tile, inserted) = Node::entry(root, coord, packed(coord), 60, &mut create);
        self.len += usize::from(inserted);
        tile
    }

    pub(crate) fn insert(&mut self, coord: TileCoord, tile: Arc<Tile>) -> Option<Arc<Tile>> {
        let existed = self.contains_key(&coord);
        let stored = self.get_or_insert_with(coord, || tile.clone());
        let old = std::mem::replace(stored, tile);
        existed.then_some(old)
    }

    pub(crate) fn remove(&mut self, coord: &TileCoord) -> Option<Arc<Tile>> {
        self.get(coord)?;
        let (removed, empty) = Node::remove(self.root.as_mut()?, *coord, packed(*coord), 60);
        self.len -= usize::from(removed.is_some());
        if empty {
            self.root = None;
        }
        removed
    }

    pub(crate) fn iter(&self) -> Iter<'_> {
        Iter { next: self.root.as_deref(), parents: std::array::from_fn(|_| [].iter()), depth: 0 }
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &TileCoord> {
        self.iter().map(|(coord, _)| coord)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&TileCoord, &Arc<Tile>) -> bool) {
        let removed: Vec<_> = self.iter().filter_map(|(coord, tile)| (!keep(coord, tile)).then_some(*coord)).collect();
        for coord in removed {
            self.remove(&coord);
        }
    }
}

pub(crate) struct Iter<'a> {
    next: Option<&'a Node>,
    parents: [std::slice::Iter<'a, Arc<Node>>; 16],
    depth: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a TileCoord, &'a Arc<Tile>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.next.take() {
                Some(Node::Leaf(coord, tile)) => return Some((coord, tile)),
                Some(Node::Branch { children, .. }) => {
                    *self.parents.get_mut(self.depth)? = children.iter();
                    self.depth += 1;
                }
                None => {}
            }
            while self.depth > 0 {
                self.next = self.parents.get_mut(self.depth - 1)?.next().map(Arc::as_ref);
                if self.next.is_some() {
                    break;
                }
                self.depth -= 1;
            }
            self.next?;
        }
    }
}

impl<'a> IntoIterator for &'a TileDir {
    type Item = (&'a TileCoord, &'a Arc<Tile>);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<(TileCoord, Arc<Tile>)> for TileDir {
    fn from_iter<I: IntoIterator<Item = (TileCoord, Arc<Tile>)>>(iter: I) -> Self {
        let mut directory = Self::new();
        for (coord, tile) in iter {
            directory.insert(coord, tile);
        }
        directory
    }
}

#[cfg(test)]
mod tests;
