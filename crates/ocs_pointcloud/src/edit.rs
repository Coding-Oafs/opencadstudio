//! Sparse, source-indexed point edits and undoable transactions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// Attributes that can be changed without copying the source point cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PointPatch {
    pub classification: Option<u8>,
    pub synthetic: Option<bool>,
    pub key_point: Option<bool>,
    pub withheld: Option<bool>,
    pub overlap: Option<bool>,
    /// Replacement survey elevation. `None` preserves the source Z value.
    pub elevation: Option<f64>,
}

impl PointPatch {
    pub fn classification(value: u8) -> Self {
        Self {
            classification: Some(value),
            ..Self::default()
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }

    /// Applies the fields present in `newer`, preserving every other field.
    pub fn merge(self, newer: Self) -> Self {
        Self {
            classification: newer.classification.or(self.classification),
            synthetic: newer.synthetic.or(self.synthetic),
            key_point: newer.key_point.or(self.key_point),
            withheld: newer.withheld.or(self.withheld),
            overlap: newer.overlap.or(self.overlap),
            elevation: newer.elevation.or(self.elevation),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditTransaction {
    pub id: u64,
    pub created_unix_ms: u64,
    pub label: String,
    pub affected_points: usize,
    before: Vec<(u64, Option<PointPatch>)>,
    after: Vec<(u64, PointPatch)>,
}

/// Sparse edit overlay keyed by the stable zero-based index in the source LAS.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditStore {
    changes: BTreeMap<u64, PointPatch>,
    committed: Vec<EditTransaction>,
    #[serde(skip)]
    redo: Vec<EditTransaction>,
    next_transaction_id: u64,
}

impl EditStore {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn transaction_count(&self) -> usize {
        self.committed.len()
    }

    pub fn patch_for(&self, source_index: u64) -> Option<PointPatch> {
        self.changes.get(&source_index).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, PointPatch)> + '_ {
        self.changes.iter().map(|(&index, &patch)| (index, patch))
    }

    pub fn transactions(&self) -> &[EditTransaction] {
        &self.committed
    }

    /// Applies one patch to a deduplicated set as a single audit/undo unit.
    pub fn apply(
        &mut self,
        label: impl Into<String>,
        source_indices: impl IntoIterator<Item = u64>,
        patch: PointPatch,
    ) -> usize {
        if patch.is_empty() {
            return 0;
        }
        let unique: BTreeSet<_> = source_indices.into_iter().collect();
        if unique.is_empty() {
            return 0;
        }

        let mut before = Vec::with_capacity(unique.len());
        let mut after = Vec::with_capacity(unique.len());
        for source_index in unique {
            let old = self.changes.get(&source_index).copied();
            let merged = old.unwrap_or_default().merge(patch);
            before.push((source_index, old));
            after.push((source_index, merged));
            self.changes.insert(source_index, merged);
        }

        self.next_transaction_id = self.next_transaction_id.saturating_add(1).max(1);
        self.committed.push(EditTransaction {
            id: self.next_transaction_id,
            created_unix_ms: unix_ms(),
            label: label.into(),
            affected_points: before.len(),
            before,
            after,
        });
        self.redo.clear();
        self.committed
            .last()
            .map_or(0, |transaction| transaction.affected_points)
    }

    pub fn undo(&mut self) -> Option<&EditTransaction> {
        let transaction = self.committed.pop()?;
        for (source_index, previous) in &transaction.before {
            match previous {
                Some(patch) => {
                    self.changes.insert(*source_index, *patch);
                }
                None => {
                    self.changes.remove(source_index);
                }
            }
        }
        self.redo.push(transaction);
        self.redo.last()
    }

    pub fn redo(&mut self) -> Option<&EditTransaction> {
        let transaction = self.redo.pop()?;
        for (source_index, patch) in &transaction.after {
            self.changes.insert(*source_index, *patch);
        }
        self.committed.push(transaction);
        self.committed.last()
    }

    /// Rebuilds transient state after deserializing an older sidecar.
    pub fn normalize_after_load(&mut self) {
        self.redo.clear();
        self.next_transaction_id = self
            .committed
            .iter()
            .map(|transaction| transaction.id)
            .max()
            .unwrap_or(0)
            .max(self.next_transaction_id);
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
