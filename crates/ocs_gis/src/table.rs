//! Attribute-table operations shared by the UI, Python, and workflows.

use crate::{Feature, FeatureLayer, FieldValue};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Contains,
    IsNull,
    IsNotNull,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttributeQuery {
    pub field: String,
    pub operator: CompareOp,
    pub value: FieldValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortField {
    pub field: String,
    pub direction: SortDirection,
}

/// A mutable attribute-table view with stable feature-id selection.
pub struct AttributeTable<'a> {
    layer: &'a mut FeatureLayer,
    selected: BTreeSet<u64>,
}

impl<'a> AttributeTable<'a> {
    pub fn new(layer: &'a mut FeatureLayer) -> Self {
        Self {
            layer,
            selected: BTreeSet::new(),
        }
    }

    pub fn selected(&self) -> &BTreeSet<u64> {
        &self.selected
    }

    pub fn select_where(&mut self, queries: &[AttributeQuery]) -> usize {
        self.selected = self
            .layer
            .features
            .iter()
            .filter(|feature| queries.iter().all(|query| matches_query(feature, query)))
            .map(|feature| feature.id)
            .collect();
        self.selected.len()
    }

    pub fn select_ids(&mut self, ids: impl IntoIterator<Item = u64>) -> usize {
        let available: BTreeSet<u64> = self
            .layer
            .features
            .iter()
            .map(|feature| feature.id)
            .collect();
        self.selected = ids
            .into_iter()
            .filter(|id| available.contains(id))
            .collect();
        self.selected.len()
    }

    /// Return feature ids in a deterministic multi-field sort order.
    pub fn sorted_ids(&self, fields: &[SortField]) -> Vec<u64> {
        let mut features: Vec<&Feature> = self.layer.features.iter().collect();
        features.sort_by(|left, right| {
            for field in fields {
                let ordering = compare_values(
                    left.properties
                        .get(&field.field)
                        .unwrap_or(&FieldValue::Null),
                    right
                        .properties
                        .get(&field.field)
                        .unwrap_or(&FieldValue::Null),
                );
                let ordering = match field.direction {
                    SortDirection::Ascending => ordering,
                    SortDirection::Descending => ordering.reverse(),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.id.cmp(&right.id)
        });
        features.into_iter().map(|feature| feature.id).collect()
    }

    /// Calculate or replace one field for the selected rows (all rows when
    /// no selection is active). The callback makes this usable from native
    /// dialogs and script workers without embedding an expression language.
    pub fn calculate_field<F>(&mut self, field: &str, mut calculate: F) -> usize
    where
        F: FnMut(&Feature) -> FieldValue,
    {
        if !self.layer.fields.iter().any(|existing| existing == field) {
            self.layer.fields.push(field.to_string());
        }
        let selected = self.selected.clone();
        let all = selected.is_empty();
        let updates: BTreeMap<u64, FieldValue> = self
            .layer
            .features
            .iter()
            .filter(|feature| all || selected.contains(&feature.id))
            .map(|feature| (feature.id, calculate(feature)))
            .collect();
        for feature in &mut self.layer.features {
            if let Some(value) = updates.get(&feature.id) {
                feature.properties.insert(field.to_string(), value.clone());
            }
        }
        updates.len()
    }

    /// Join attributes by exact key, preserving feature geometry and ids.
    pub fn join(
        &mut self,
        feature_key: &str,
        rows: &[BTreeMap<String, FieldValue>],
        row_key: &str,
        prefix: &str,
    ) -> usize {
        let index: Vec<(&FieldValue, &BTreeMap<String, FieldValue>)> = rows
            .iter()
            .filter_map(|row| row.get(row_key).map(|key| (key, row)))
            .collect();
        let mut changed = 0;
        for feature in &mut self.layer.features {
            let Some(key) = feature.properties.get(feature_key) else {
                continue;
            };
            let Some((_, row)) = index.iter().find(|(candidate, _)| *candidate == key) else {
                continue;
            };
            for (name, value) in row.iter().filter(|(name, _)| name.as_str() != row_key) {
                let joined_name = format!("{prefix}{name}");
                if !self.layer.fields.contains(&joined_name) {
                    self.layer.fields.push(joined_name.clone());
                }
                feature.properties.insert(joined_name, value.clone());
            }
            changed += 1;
        }
        changed
    }
}

fn matches_query(feature: &Feature, query: &AttributeQuery) -> bool {
    let actual = feature
        .properties
        .get(&query.field)
        .unwrap_or(&FieldValue::Null);
    let ordering = compare_values(actual, &query.value);
    match query.operator {
        CompareOp::Equal => actual == &query.value,
        CompareOp::NotEqual => actual != &query.value,
        CompareOp::Less => ordering == Ordering::Less,
        CompareOp::LessOrEqual => ordering != Ordering::Greater,
        CompareOp::Greater => ordering == Ordering::Greater,
        CompareOp::GreaterOrEqual => ordering != Ordering::Less,
        CompareOp::Contains => value_text(actual)
            .to_ascii_lowercase()
            .contains(&value_text(&query.value).to_ascii_lowercase()),
        CompareOp::IsNull => matches!(actual, FieldValue::Null),
        CompareOp::IsNotNull => !matches!(actual, FieldValue::Null),
    }
}

fn value_text(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(value) => value.clone(),
        other => other.to_sql_text(),
    }
}

fn compare_values(left: &FieldValue, right: &FieldValue) -> Ordering {
    match (left, right) {
        (FieldValue::Integer(left), FieldValue::Integer(right)) => left.cmp(right),
        (FieldValue::Integer(left), FieldValue::Real(right)) => (*left as f64).total_cmp(right),
        (FieldValue::Real(left), FieldValue::Integer(right)) => left.total_cmp(&(*right as f64)),
        (FieldValue::Real(left), FieldValue::Real(right)) => left.total_cmp(right),
        (FieldValue::Boolean(left), FieldValue::Boolean(right)) => left.cmp(right),
        (FieldValue::Null, FieldValue::Null) => Ordering::Equal,
        (FieldValue::Null, _) => Ordering::Less,
        (_, FieldValue::Null) => Ordering::Greater,
        _ => value_text(left).cmp(&value_text(right)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Geometry;

    fn layer() -> FeatureLayer {
        let mut layer = FeatureLayer::new("parcels", 4326);
        for (name, area) in [("A", 50), ("B", 125), ("C", 75)] {
            layer.push(
                Geometry::Point([area as f64, 0.0]),
                BTreeMap::from([
                    ("name".into(), FieldValue::Text(name.into())),
                    ("area".into(), FieldValue::Integer(area)),
                ]),
            );
        }
        layer
    }

    #[test]
    fn filters_sorts_calculates_and_joins_by_stable_id() {
        let mut layer = layer();
        let mut table = AttributeTable::new(&mut layer);
        assert_eq!(
            table.select_where(&[AttributeQuery {
                field: "area".into(),
                operator: CompareOp::Greater,
                value: FieldValue::Integer(60),
            }]),
            2
        );
        assert_eq!(
            table.calculate_field("double", |feature| {
                match feature.properties["area"] {
                    FieldValue::Integer(value) => FieldValue::Integer(value * 2),
                    _ => FieldValue::Null,
                }
            }),
            2
        );
        assert_eq!(
            table.sorted_ids(&[SortField {
                field: "area".into(),
                direction: SortDirection::Descending,
            }]),
            vec![2, 3, 1]
        );
        let rows = vec![BTreeMap::from([
            ("code".into(), FieldValue::Text("B".into())),
            ("zone".into(), FieldValue::Text("commercial".into())),
        ])];
        assert_eq!(table.join("name", &rows, "code", "join_"), 1);
        assert_eq!(
            layer.features[1].properties["join_zone"],
            FieldValue::Text("commercial".into())
        );
        assert!(!layer.features[0].properties.contains_key("double"));
    }
}
