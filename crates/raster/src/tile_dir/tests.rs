use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;

fn tile(value: u8) -> Arc<Tile> {
    Arc::new(Tile { data: vec![value].into_boxed_slice() })
}

fn coordinate() -> impl Strategy<Value = i32> {
    prop_oneof![4 => -20i32..20, 1 => any::<i32>(), 1 => prop::sample::select(vec![i32::MIN, i32::MAX, -1, 0, 1])]
}

proptest! {
    #[test]
    fn edits_match_ordered_model_and_preserve_snapshots(
        edits in prop::collection::vec((coordinate(), coordinate(), prop::option::of(any::<u8>())), 0..128)
    ) {
        let mut directory = TileDir::new();
        let mut model = BTreeMap::new();
        let mut snapshots = Vec::new();
        for (tx, ty, value) in edits {
            snapshots.push((directory.clone(), model.clone()));
            let coord = TileCoord::new(tx, ty);
            match value {
                Some(value) => {
                    let value = tile(value);
                    prop_assert_eq!(directory.insert(coord, value.clone()), model.insert(coord, value));
                }
                None => prop_assert_eq!(directory.remove(&coord), model.remove(&coord)),
            }
            prop_assert_eq!(directory.get(&coord), model.get(&coord));
            prop_assert_eq!(directory.len(), model.len());
            prop_assert_eq!(directory.is_empty(), model.is_empty());
            prop_assert!(directory.iter().eq(model.iter()));
            let rebuilt: TileDir = model.iter().rev().map(|(coord, value)| (*coord, value.clone())).collect();
            prop_assert_eq!(&directory, &rebuilt);
        }
        for (snapshot, expected) in snapshots {
            prop_assert!(snapshot.iter().eq(expected.iter()));
        }
    }

    #[test]
    fn repeated_edits_and_pruning_keep_canonical_shape(
        edits in prop::collection::vec((0i32..32, prop::option::of(any::<u8>())), 0..256)
    ) {
        let mut directory = TileDir::new();
        let mut model = BTreeMap::new();
        for (ty, value) in edits {
            let coord = TileCoord::new(-1, ty);
            match value {
                Some(value) => {
                    directory.insert(coord, tile(value));
                    model.insert(coord, tile(value));
                }
                None => { directory.remove(&coord); model.remove(&coord); }
            }
            prop_assert!(directory.iter().eq(model.iter()));
        }
        directory.retain(|coord, _| coord.ty % 2 == 0);
        model.retain(|coord, _| coord.ty % 2 == 0);
        let rebuilt: TileDir = model.into_iter().collect();
        prop_assert_eq!(&directory, &rebuilt);
        for coord in directory.keys().copied().collect::<Vec<_>>() {
            directory.remove(&coord);
        }
        prop_assert_eq!(directory, TileDir::new());
    }
}

#[test]
fn clones_share_roots_and_mutations_copy_only_the_path() {
    let mut directory: TileDir = [TileCoord::new(i32::MIN, 0), TileCoord::new(0, 0), TileCoord::new(0, 1)].into_iter().map(|coord| (coord, tile(1))).collect();
    let snapshot = directory.clone();
    assert!(Arc::ptr_eq(directory.root.as_ref().unwrap(), snapshot.root.as_ref().unwrap()));
    let coord = TileCoord::new(0, 1);
    assert_eq!(Arc::strong_count(directory.get(&coord).unwrap()), 1);
    Arc::make_mut(directory.get_mut(&coord).unwrap()).data[0] = 9;
    assert_eq!(snapshot.get(&coord).unwrap().data[0], 1);
    assert_eq!(directory.get(&coord).unwrap().data[0], 9);
    let (Node::Branch { children, .. }, Node::Branch { children: old, .. }) = (directory.root.as_deref().unwrap(), snapshot.root.as_deref().unwrap()) else {
        panic!("expected branching roots")
    };
    assert!(Arc::ptr_eq(&children[0], &old[0]));
    assert_ne!(directory, snapshot);
    directory.insert(coord, tile(1));
    assert_eq!(directory, snapshot);
}

#[test]
fn signed_extremes_and_last_nibble_splits_stay_sorted() {
    let coords = [i32::MIN, -1, 0, 1, 15, 16, i32::MAX];
    let model: BTreeMap<_, _> = coords.into_iter().flat_map(|tx| coords.into_iter().map(move |ty| (TileCoord::new(tx, ty), tile(7)))).collect();
    let mut directory: TileDir = model.iter().rev().map(|(coord, tile)| (*coord, tile.clone())).collect();
    assert!(directory.iter().eq(model.iter()));
    assert_eq!(format!("{directory:?}"), format!("{model:?}"));
    let before = directory.clone();
    assert!(directory.get_mut(&TileCoord::new(2, 3)).is_none());
    assert!(directory.remove(&TileCoord::new(2, 3)).is_none());
    assert!(Arc::ptr_eq(directory.root.as_ref().unwrap(), before.root.as_ref().unwrap()));
    for coord in model.keys().skip(1) {
        directory.remove(coord);
    }
    let singleton: TileDir = model.into_iter().take(1).collect();
    assert_eq!(directory, singleton);
}
