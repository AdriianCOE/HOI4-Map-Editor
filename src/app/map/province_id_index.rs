//! Read-only lookup support for preserved province IDs.
//!
//! This index intentionally supports sparse IDs internally. The map loader still
//! owns the current contiguous-ID compatibility gate; relaxing that gate is a
//! separate migration step.

use ahash::AHashMap;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::Color;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvinceIdIndexError {
    InvalidIdZero,
    DuplicateId(u32),
    DuplicateColor(Color),
    AllocationOverflow,
}

impl fmt::Display for ProvinceIdIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdZero => write!(f, "province ID 0 is reserved"),
            Self::DuplicateId(id) => write!(f, "duplicate province ID {id}"),
            Self::DuplicateColor(color) => write!(f, "duplicate province color {color:?}"),
            Self::AllocationOverflow => write!(f, "no province ID is available after u32::MAX"),
        }
    }
}

impl Error for ProvinceIdIndexError {}

/// Bidirectional, deterministic-by-ID index of preserved province identities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvinceIdIndex {
    by_id: BTreeMap<u32, Color>,
    by_color: AHashMap<Color, u32>,
}

impl ProvinceIdIndex {
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (u32, Color)>,
    ) -> Result<Self, ProvinceIdIndexError> {
        let mut index = Self::default();
        for (id, color) in pairs {
            index.insert(id, color)?;
        }
        Ok(index)
    }

    pub fn insert(&mut self, id: u32, color: Color) -> Result<(), ProvinceIdIndexError> {
        if id == 0 {
            return Err(ProvinceIdIndexError::InvalidIdZero);
        }
        if self.by_id.contains_key(&id) {
            return Err(ProvinceIdIndexError::DuplicateId(id));
        }
        if self.by_color.contains_key(&color) {
            return Err(ProvinceIdIndexError::DuplicateColor(color));
        }
        self.by_id.insert(id, color);
        self.by_color.insert(color, id);
        Ok(())
    }

    pub fn color_for_id(&self, id: u32) -> Option<Color> {
        self.by_id.get(&id).copied()
    }

    pub fn id_for_color(&self, color: Color) -> Option<u32> {
        self.by_color.get(&color).copied()
    }

    pub fn contains_id(&self, id: u32) -> bool {
        self.by_id.contains_key(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, Color)> + '_ {
        self.by_id.iter().map(|(&id, &color)| (id, color))
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_id.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn province_count(&self) -> usize {
        self.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn min_id(&self) -> Option<u32> {
        self.by_id.keys().next().copied()
    }

    pub fn min_province_id(&self) -> Option<u32> {
        self.min_id()
    }

    pub fn max_id(&self) -> Option<u32> {
        self.by_id.keys().next_back().copied()
    }

    pub fn max_province_id(&self) -> Option<u32> {
        self.max_id()
    }

    pub fn is_contiguous_from_one(&self) -> bool {
        self.ids()
            .enumerate()
            .all(|(offset, id)| id == offset as u32 + 1)
    }

    /// Pure allocation policy for a future sparse-ID writer: `max + 1`.
    pub fn next_allocatable_id(&self) -> Result<u32, ProvinceIdIndexError> {
        self.max_id()
            .map(|id| {
                id.checked_add(1)
                    .ok_or(ProvinceIdIndexError::AllocationOverflow)
            })
            .unwrap_or(Ok(1))
    }

    pub(crate) fn remove_color(&mut self, color: Color) {
        if let Some(id) = self.by_color.remove(&color) {
            self.by_id.remove(&id);
        }
    }

    pub(crate) fn rekey_color(&mut self, previous: Color, replacement: Color) {
        if let Some(id) = self.by_color.remove(&previous) {
            let old = self.by_color.insert(replacement, id);
            debug_assert_eq!(old, None);
            self.by_id.insert(id, replacement);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProvinceIdIndex, ProvinceIdIndexError};

    #[test]
    fn indexes_contiguous_sparse_and_high_ids_bidirectionally() {
        let index = ProvinceIdIndex::from_pairs([
            (42, [4, 2, 0]),
            (1, [1, 0, 0]),
            (500, [5, 0, 0]),
            (7, [0, 7, 0]),
        ])
        .unwrap();

        assert_eq!(index.province_count(), 4);
        assert_eq!(index.min_province_id(), Some(1));
        assert_eq!(index.max_province_id(), Some(500));
        assert!(index.contains_id(42));
        assert_eq!(index.color_for_id(7), Some([0, 7, 0]));
        assert_eq!(index.id_for_color([5, 0, 0]), Some(500));
        assert!(!index.is_contiguous_from_one());
    }

    #[test]
    fn iterates_pairs_in_ascending_id_order() {
        let index =
            ProvinceIdIndex::from_pairs([(3, [3, 0, 0]), (1, [1, 0, 0]), (2, [2, 0, 0])]).unwrap();
        assert_eq!(
            index.iter().collect::<Vec<_>>(),
            vec![(1, [1, 0, 0]), (2, [2, 0, 0]), (3, [3, 0, 0])]
        );
        assert!(index.is_contiguous_from_one());
    }

    #[test]
    fn empty_index_has_no_bounds_and_allocates_one() {
        let index = ProvinceIdIndex::default();
        assert!(index.is_empty());
        assert_eq!(index.province_count(), 0);
        assert_eq!(index.min_province_id(), None);
        assert_eq!(index.max_province_id(), None);
        assert_eq!(index.next_allocatable_id(), Ok(1));
    }

    #[test]
    fn supports_high_ids_without_using_them_as_dense_indexes() {
        let index = ProvinceIdIndex::from_pairs([(1, [1, 0, 0]), (10_000, [0, 1, 0])]).unwrap();
        assert_eq!(index.province_count(), 2);
        assert_eq!(index.max_province_id(), Some(10_000));
        assert_eq!(index.next_allocatable_id(), Ok(10_001));
    }

    #[test]
    fn rejects_duplicate_ids_and_colors() {
        assert_eq!(
            ProvinceIdIndex::from_pairs([(0, [1, 0, 0])]),
            Err(ProvinceIdIndexError::InvalidIdZero)
        );
        assert_eq!(
            ProvinceIdIndex::from_pairs([(1, [1, 0, 0]), (1, [2, 0, 0])]),
            Err(ProvinceIdIndexError::DuplicateId(1))
        );
        assert_eq!(
            ProvinceIdIndex::from_pairs([(1, [1, 0, 0]), (2, [1, 0, 0])]),
            Err(ProvinceIdIndexError::DuplicateColor([1, 0, 0]))
        );
    }

    #[test]
    fn next_allocation_uses_maximum_and_reports_overflow() {
        let index = ProvinceIdIndex::from_pairs([(7, [7, 0, 0]), (42, [4, 2, 0])]).unwrap();
        assert_eq!(index.next_allocatable_id(), Ok(43));

        let exhausted = ProvinceIdIndex::from_pairs([(u32::MAX, [1, 2, 3])]).unwrap();
        assert_eq!(
            exhausted.next_allocatable_id(),
            Err(ProvinceIdIndexError::AllocationOverflow)
        );
    }
}
