//! Generation-checked handles shared with Java.
//!
//! A handle is never a pointer. The low 32 bits hold a slot plus one (zero is
//! reserved for `None`), and the high 32 bits hold a generation. Removing or
//! clearing a value advances its generation, so a Java object retained after
//! a world closes cannot alias a later Rust object.

use serde::{Deserialize, Serialize};

/// An opaque, nonzero handle a Java shim may retain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Handle(u64);

impl Handle {
    fn new(slot: u32, generation: u32) -> Self {
        debug_assert!(generation != 0);
        Self((u64::from(generation) << 32) | u64::from(slot + 1))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn from_raw(value: u64) -> Option<Self> {
        let handle = Self(value);
        (value != 0 && handle.generation() != 0 && handle.slot().is_some()).then_some(handle)
    }

    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub fn slot(self) -> Option<u32> {
        let encoded = self.0 as u32;
        encoded.checked_sub(1)
    }
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A bounded table of Rust-owned values addressed by [`Handle`].
#[derive(Debug)]
pub struct HandleTable<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    limit: usize,
    len: usize,
}

impl<T> HandleTable<T> {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            limit,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert(&mut self, value: T) -> Result<Handle, T> {
        if self.len == self.limit {
            return Err(value);
        }
        let slot = if let Some(slot) = self.free.pop() {
            slot
        } else {
            let Ok(slot) = u32::try_from(self.slots.len()) else {
                return Err(value);
            };
            self.slots.push(Slot {
                generation: 1,
                value: None,
            });
            slot
        };
        let entry = &mut self.slots[slot as usize];
        debug_assert!(entry.value.is_none());
        entry.value = Some(value);
        self.len += 1;
        Ok(Handle::new(slot, entry.generation))
    }

    pub fn get(&self, handle: Handle) -> Option<&T> {
        let slot = handle.slot()? as usize;
        let entry = self.slots.get(slot)?;
        (entry.generation == handle.generation())
            .then_some(entry.value.as_ref())
            .flatten()
    }

    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let slot = handle.slot()? as usize;
        let entry = self.slots.get_mut(slot)?;
        (entry.generation == handle.generation())
            .then_some(entry.value.as_mut())
            .flatten()
    }

    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let slot = handle.slot()? as usize;
        let entry = self.slots.get_mut(slot)?;
        if entry.generation != handle.generation() {
            return None;
        }
        let value = entry.value.take()?;
        entry.generation = next_generation(entry.generation);
        self.free.push(slot as u32);
        self.len -= 1;
        Some(value)
    }

    /// Invalidates every outstanding handle, including slots already free.
    pub fn clear(&mut self) {
        self.free.clear();
        for (slot, entry) in self.slots.iter_mut().enumerate() {
            entry.value = None;
            entry.generation = next_generation(entry.generation);
            self.free.push(slot as u32);
        }
        self.len = 0;
    }
}

fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_handles_never_alias_reused_slots() {
        let mut table = HandleTable::with_limit(2);
        let first = table.insert("first").unwrap();
        assert_eq!(table.get(first), Some(&"first"));
        assert_eq!(table.remove(first), Some("first"));
        assert_eq!(table.get(first), None);

        let second = table.insert("second").unwrap();
        assert_eq!(first.slot(), second.slot());
        assert_ne!(first.generation(), second.generation());
        assert_eq!(table.get(first), None);
        assert_eq!(table.get(second), Some(&"second"));
    }

    #[test]
    fn clearing_a_world_invalidates_all_handles() {
        let mut table = HandleTable::with_limit(2);
        let first = table.insert(1).unwrap();
        let second = table.insert(2).unwrap();
        table.clear();
        assert!(table.is_empty());
        assert_eq!(table.get(first), None);
        assert_eq!(table.get(second), None);
        let replacement = table.insert(3).unwrap();
        assert_ne!(replacement, second);
    }

    #[test]
    fn zero_and_malformed_raw_handles_are_refused() {
        assert_eq!(Handle::from_raw(0), None);
        assert_eq!(Handle::from_raw(1), None);
        assert!(Handle::from_raw((1_u64 << 32) | 1).is_some());
    }

    #[test]
    fn the_table_enforces_its_bound() {
        let mut table = HandleTable::with_limit(1);
        table.insert(1).unwrap();
        assert_eq!(table.insert(2), Err(2));
    }
}
