//! Tests VecMap through its public API. The sibling module prevents access to private state.

use crate::vecmap::*;
use std::{cell::Cell, rc::Rc};

#[test]
fn test_entry_insert_and_update() {
    let mut map: VecMap<&str, i32> = VecMap::new();
    assert_eq!(*map.entry("a").key(), "a");
    assert_eq!(*map.entry("a").or_insert(1), 1);
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &1)]);
    assert_eq!(*map.entry("a").key(), "a");
    assert_eq!(*map.entry("a").or_insert(99), 1);
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &1)]);

    let mut map: VecMap<&str, i32> = VecMap::new();
    *map.entry("a").or_insert(0) = 5;
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &5)]);
}

#[test]
fn test_entry_lazy_insert() {
    let mut map: VecMap<&str, i32> = VecMap::new();
    map.entry("a").or_insert_with(|| 42);
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &42)]);

    map.entry("a")
        .or_insert_with(|| panic!("should not be called"));
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &42)]);
}

#[test]
fn test_entry_or_insert_with_key() {
    let mut map: VecMap<&str, String> = VecMap::new();
    map.entry("hello").or_insert_with_key(|k| k.to_uppercase());
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        vec![(&"hello", &"HELLO".to_string())]
    );
}

#[test]
fn test_entry_or_insert_default() {
    let mut map: VecMap<&str, i32> = VecMap::new();
    map.entry("a").or_insert_default();
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &0)]);
}

#[test]
fn test_entries_preserve_order_and_are_independent() {
    let mut map: VecMap<&str, i32> = VecMap::new();
    map.entry("b").or_insert(2);
    map.entry("a").or_insert(1);
    map.entry("c").or_insert(3);
    assert_eq!(map.iter().count(), 3);

    map.entry("a").or_insert(99);
    assert_eq!(map.iter().count(), 3);
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        vec![(&"b", &2), (&"a", &1), (&"c", &3)]
    );
}

#[derive(PartialEq, Eq)]
struct CountedKey {
    value: String,
    clone_count: Rc<Cell<usize>>,
}

impl Clone for CountedKey {
    fn clone(&self) -> Self {
        self.clone_count.set(self.clone_count.get() + 1);
        CountedKey {
            value: self.value.clone(),
            clone_count: self.clone_count.clone(),
        }
    }
}

#[test]
fn test_entry_ref_insert_and_update() {
    let mut map: VecMap<String, i32> = VecMap::new();
    let key = "a".to_string();
    assert_eq!(*map.entry_ref(&key).key(), key);
    assert_eq!(*map.entry_ref(&key).or_insert(1), 1);
    assert_eq!(map.iter().count(), 1);
    assert_eq!(*map.entry_ref(&key).key(), key);
    assert_eq!(*map.entry_ref(&key).or_insert(99), 1);
    assert_eq!(map.iter().count(), 1);

    let mut map: VecMap<String, i32> = VecMap::new();
    *map.entry_ref(&key).or_insert(0) = 5;
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&key, &5)]);
}

#[test]
fn test_entry_ref_clones_key_only_on_insert() {
    let clone_count = Rc::new(Cell::new(0));
    let key = CountedKey {
        value: "a".to_string(),
        clone_count: clone_count.clone(),
    };
    let mut map: VecMap<CountedKey, i32> = VecMap::new();
    map.entry_ref(&key).or_insert(1);
    assert_eq!(clone_count.get(), 1);

    map.entry_ref(&key).or_insert(99);
    assert_eq!(clone_count.get(), 1);
}

#[test]
fn test_entry_ref_or_insert_with_key() {
    let mut map: VecMap<String, String> = VecMap::new();
    let key = "hello".to_string();
    map.entry_ref(&key).or_insert_with_key(|k| k.to_uppercase());
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        vec![(&"hello".to_string(), &"HELLO".to_string())]
    );
}

#[test]
fn test_entry_ref_or_insert_with_not_called_when_occupied() {
    let mut map: VecMap<String, i32> = VecMap::new();
    let key = "a".to_string();
    map.entry_ref(&key).or_insert(1);
    map.entry_ref(&key)
        .or_insert_with(|| panic!("should not be called"));
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&key, &1)]);
}

#[test]
fn test_entry_ref_or_insert_default() {
    let mut map: VecMap<String, i32> = VecMap::new();
    map.entry_ref(&"a".to_string()).or_insert_default();
    assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a".to_string(), &0)]);
}
