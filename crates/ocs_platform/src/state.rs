use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ObjectId {
    CadEntity { document: String, entity: u64 },
    Feature { layer: String, feature: u64 },
    Point { source: String, record: u64 },
    Raster { source: String },
    Surface { source: String },
    Mesh { source: String },
    ModelObject { module: String, id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOperation {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectChange {
    pub object: ObjectId,
    pub operation: ChangeOperation,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl ObjectChange {
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self.operation {
            ChangeOperation::Create => self.before.is_none() && self.after.is_some(),
            ChangeOperation::Update => self.before.is_some() && self.after.is_some(),
            ChangeOperation::Delete => self.before.is_some() && self.after.is_none(),
        };
        valid.then_some(()).ok_or_else(|| {
            format!(
                "{:?} has inconsistent before/after snapshots",
                self.operation
            )
        })
    }

    pub fn inverse(&self) -> Self {
        Self {
            object: self.object.clone(),
            operation: match self.operation {
                ChangeOperation::Create => ChangeOperation::Delete,
                ChangeOperation::Update => ChangeOperation::Update,
                ChangeOperation::Delete => ChangeOperation::Create,
            },
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    Committed,
    Reverted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnifiedTransaction {
    pub id: String,
    pub name: String,
    pub tool_id: String,
    pub created_unix_ms: u64,
    pub changes: Vec<ObjectChange>,
    pub status: TransactionStatus,
    pub metadata: BTreeMap<String, Value>,
}

impl UnifiedTransaction {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        tool_id: impl Into<String>,
        changes: Vec<ObjectChange>,
    ) -> Result<Self, String> {
        let transaction = Self {
            id: id.into(),
            name: name.into(),
            tool_id: tool_id.into(),
            created_unix_ms: unix_ms(),
            changes,
            status: TransactionStatus::Committed,
            metadata: BTreeMap::new(),
        };
        transaction.validate()?;
        Ok(transaction)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.tool_id.trim().is_empty()
        {
            return Err("transaction id, name, and tool id must not be empty".into());
        }
        if self.changes.is_empty() {
            return Err("a transaction must contain at least one change".into());
        }
        let mut objects = BTreeSet::new();
        for change in &self.changes {
            change.validate()?;
            if !objects.insert(&change.object) {
                return Err("an object can appear only once in a transaction".into());
            }
        }
        Ok(())
    }

    pub fn inverse(&self, id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: format!("Undo {}", self.name),
            tool_id: "platform.transaction.undo".into(),
            created_unix_ms: unix_ms(),
            changes: self
                .changes
                .iter()
                .rev()
                .map(ObjectChange::inverse)
                .collect(),
            status: TransactionStatus::Committed,
            metadata: BTreeMap::from([("reverts".into(), Value::String(self.id.clone()))]),
        }
    }
}

/// Adapter implemented by each data model. Transactions preflight all
/// snapshots before mutating, then roll back already-applied changes if an
/// adapter rejects a later change.
pub trait TransactionStore {
    fn snapshot(&self, object: &ObjectId) -> Result<Option<Value>, String>;
    fn write(&mut self, object: &ObjectId, value: Value) -> Result<(), String>;
    fn delete(&mut self, object: &ObjectId) -> Result<(), String>;
}

pub fn apply_transaction(
    store: &mut dyn TransactionStore,
    transaction: &UnifiedTransaction,
) -> Result<(), String> {
    transaction.validate()?;
    for change in &transaction.changes {
        let actual = store.snapshot(&change.object)?;
        if actual != change.before {
            return Err(format!(
                "transaction precondition failed for {:?}",
                change.object
            ));
        }
    }
    let mut applied: Vec<&ObjectChange> = Vec::new();
    for change in &transaction.changes {
        let result = match &change.after {
            Some(value) => store.write(&change.object, value.clone()),
            None => store.delete(&change.object),
        };
        if let Err(error) = result {
            for previous in applied.into_iter().rev() {
                match &previous.before {
                    Some(value) => {
                        let _ = store.write(&previous.object, value.clone());
                    }
                    None => {
                        let _ = store.delete(&previous.object);
                    }
                }
            }
            return Err(format!("transaction rolled back: {error}"));
        }
        applied.push(change);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub tool_id: String,
    pub parameters: Value,
    /// `parameter -> upstream node output key`, formatted `node.output`.
    pub bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub api_version: u32,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub metadata: BTreeMap<String, Value>,
}

impl WorkflowDefinition {
    pub fn execution_order<F>(&self, mut tool_exists: F) -> Result<Vec<&WorkflowNode>, String>
    where
        F: FnMut(&str) -> bool,
    {
        if self.id.trim().is_empty() || self.name.trim().is_empty() || self.api_version == 0 {
            return Err("workflow id/name and api version are required".into());
        }
        let nodes: BTreeMap<&str, &WorkflowNode> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        if nodes.len() != self.nodes.len() || nodes.contains_key("") {
            return Err("workflow node ids must be non-empty and unique".into());
        }
        for node in &self.nodes {
            if !tool_exists(&node.tool_id) {
                return Err(format!(
                    "workflow tool '{}' is not registered",
                    node.tool_id
                ));
            }
        }
        let mut incoming: BTreeMap<&str, usize> = nodes.keys().map(|id| (*id, 0)).collect();
        let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut seen_edges = BTreeSet::new();
        for edge in &self.edges {
            if !nodes.contains_key(edge.from.as_str()) || !nodes.contains_key(edge.to.as_str()) {
                return Err(format!(
                    "workflow edge {} -> {} names a missing node",
                    edge.from, edge.to
                ));
            }
            if edge.from == edge.to || !seen_edges.insert((&edge.from, &edge.to)) {
                return Err("workflow edges must be unique and cannot self-reference".into());
            }
            *incoming.get_mut(edge.to.as_str()).unwrap() += 1;
            outgoing
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
        let mut ready: VecDeque<&str> = incoming
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect();
        let mut ordered = Vec::with_capacity(nodes.len());
        while let Some(id) = ready.pop_front() {
            ordered.push(nodes[id]);
            for target in outgoing.get(id).into_iter().flatten() {
                let count = incoming.get_mut(target).unwrap();
                *count -= 1;
                if *count == 0 {
                    ready.push_back(target);
                }
            }
        }
        if ordered.len() != nodes.len() {
            return Err("workflow graph contains a cycle".into());
        }
        Ok(ordered)
    }
}

pub trait WorkflowExecutor {
    fn run_tool(&mut self, tool_id: &str, parameters: Value) -> Result<Value, String>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub workflow_id: String,
    pub started_unix_ms: u64,
    pub completed_unix_ms: u64,
    pub outputs: BTreeMap<String, Value>,
}

pub fn run_workflow<F>(
    workflow: &WorkflowDefinition,
    mut tool_exists: F,
    executor: &mut dyn WorkflowExecutor,
) -> Result<WorkflowRun, String>
where
    F: FnMut(&str) -> bool,
{
    let order = workflow.execution_order(&mut tool_exists)?;
    let started = unix_ms();
    let mut outputs = BTreeMap::new();
    for node in order {
        let mut parameters = node.parameters.clone();
        let object = parameters
            .as_object_mut()
            .ok_or_else(|| format!("workflow node '{}' parameters must be an object", node.id))?;
        for (parameter, binding) in &node.bindings {
            let (upstream, key) = binding
                .split_once('.')
                .ok_or_else(|| format!("invalid binding '{binding}'"))?;
            let value = outputs
                .get(upstream)
                .and_then(|value: &Value| value.get(key))
                .cloned()
                .ok_or_else(|| format!("binding '{binding}' did not produce a value"))?;
            object.insert(parameter.clone(), value);
        }
        let output = executor
            .run_tool(&node.tool_id, parameters)
            .map_err(|error| format!("workflow node '{}': {error}", node.id))?;
        outputs.insert(node.id.clone(), output);
    }
    Ok(WorkflowRun {
        workflow_id: workflow.id.clone(),
        started_unix_ms: started,
        completed_unix_ms: unix_ms(),
        outputs,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationRule {
    pub id: String,
    pub description: String,
    pub severity: ValidationSeverity,
    pub target_kind: String,
    pub expression: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StandardsPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tool_presets: BTreeMap<String, Value>,
    pub validation_rules: Vec<ValidationRule>,
    pub content_sha256: String,
    pub signer: Option<String>,
    pub signature: Option<String>,
}

impl StandardsPackage {
    pub fn seal(&mut self) -> Result<String, String> {
        self.content_sha256.clear();
        let signature = self.signature.take();
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        self.signature = signature;
        self.content_sha256 = digest.clone();
        Ok(digest)
    }

    pub fn verify_digest(&self) -> Result<(), String> {
        let expected = self.content_sha256.clone();
        let mut unsealed = self.clone();
        let actual = unsealed.seal()?;
        (expected == actual)
            .then_some(())
            .ok_or_else(|| "standards package content digest does not match".into())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.version.trim().is_empty()
        {
            return Err("standards package id, name, and version are required".into());
        }
        let mut ids = BTreeSet::new();
        if self.validation_rules.iter().any(|rule| {
            rule.id.trim().is_empty() || rule.description.trim().is_empty() || !ids.insert(&rule.id)
        }) {
            return Err("validation-rule ids must be non-empty and unique".into());
        }
        self.verify_digest()?;
        match (&self.signer, &self.signature) {
            (None, None) => Ok(()),
            (Some(public_key), Some(signature)) => {
                let public_key = decode_hex(public_key)?;
                let signature = decode_hex(signature)?;
                UnparsedPublicKey::new(&ED25519, public_key)
                    .verify(self.content_sha256.as_bytes(), &signature)
                    .map_err(|_| "standards package Ed25519 signature is invalid".to_string())
            }
            _ => Err("standards package signer and signature must be supplied together".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub id: String,
    pub tool_id: String,
    pub tool_api_version: u32,
    pub software_version: String,
    pub environment_digest: String,
    pub inputs: Vec<ProvenanceArtifact>,
    pub outputs: Vec<ProvenanceArtifact>,
    pub parameters: Value,
    pub transformations: Vec<Value>,
    pub started_unix_ms: u64,
    pub completed_unix_ms: u64,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceArtifact {
    pub uri: String,
    pub sha256: Option<String>,
    pub media_type: String,
    pub object_ids: BTreeSet<ObjectId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlatformState {
    pub transactions: Vec<UnifiedTransaction>,
    pub workflows: Vec<WorkflowDefinition>,
    pub workflow_runs: Vec<WorkflowRun>,
    pub standards: Vec<StandardsPackage>,
    pub provenance: Vec<ProvenanceRecord>,
    pub trusted_signers: BTreeSet<String>,
    pub validation_profiles: BTreeMap<String, Vec<String>>,
}

impl PlatformState {
    pub fn validate(&self) -> Result<(), String> {
        unique(
            self.transactions.iter().map(|value| value.id.as_str()),
            "transaction",
        )?;
        unique(
            self.workflows.iter().map(|value| value.id.as_str()),
            "workflow",
        )?;
        unique(
            self.standards.iter().map(|value| value.id.as_str()),
            "standards package",
        )?;
        unique(
            self.provenance.iter().map(|value| value.id.as_str()),
            "provenance record",
        )?;
        for transaction in &self.transactions {
            transaction.validate()?;
        }
        for standards in &self.standards {
            standards.validate()?;
            if standards.signature.is_some()
                && standards
                    .signer
                    .as_ref()
                    .is_none_or(|signer| !self.trusted_signers.contains(signer))
            {
                return Err(format!(
                    "standards package '{}' has an untrusted signer",
                    standards.id
                ));
            }
        }
        Ok(())
    }
}

fn unique<'a>(values: impl IntoIterator<Item = &'a str>, kind: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() || !seen.insert(value) {
            return Err(format!("{kind} ids must be non-empty and unique"));
        }
    }
    Ok(())
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 {
        return Err("hex value has an odd length".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| "hex value contains invalid characters".to_string())
        })
        .collect()
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    #[derive(Default)]
    struct MemoryStore(BTreeMap<ObjectId, Value>);
    impl TransactionStore for MemoryStore {
        fn snapshot(&self, object: &ObjectId) -> Result<Option<Value>, String> {
            Ok(self.0.get(object).cloned())
        }
        fn write(&mut self, object: &ObjectId, value: Value) -> Result<(), String> {
            self.0.insert(object.clone(), value);
            Ok(())
        }
        fn delete(&mut self, object: &ObjectId) -> Result<(), String> {
            self.0.remove(object);
            Ok(())
        }
    }

    struct MathExecutor;
    impl WorkflowExecutor for MathExecutor {
        fn run_tool(&mut self, tool_id: &str, parameters: Value) -> Result<Value, String> {
            match tool_id {
                "source" => Ok(serde_json::json!({"value": parameters["value"]})),
                "double" => {
                    Ok(serde_json::json!({"value": parameters["input"].as_i64().unwrap() * 2}))
                }
                _ => Err("unknown tool".into()),
            }
        }
    }

    #[test]
    fn unified_transaction_applies_and_reverts_across_object_kinds() {
        let cad = ObjectId::CadEntity {
            document: "site.dwg".into(),
            entity: 7,
        };
        let feature = ObjectId::Feature {
            layer: "lots".into(),
            feature: 3,
        };
        let transaction = UnifiedTransaction::new(
            "tx-1",
            "CAD and GIS edit",
            "edit.move",
            vec![
                ObjectChange {
                    object: cad.clone(),
                    operation: ChangeOperation::Create,
                    before: None,
                    after: Some(serde_json::json!({"x": 1})),
                },
                ObjectChange {
                    object: feature.clone(),
                    operation: ChangeOperation::Create,
                    before: None,
                    after: Some(serde_json::json!({"owner": "A"})),
                },
            ],
        )
        .unwrap();
        let mut store = MemoryStore::default();
        apply_transaction(&mut store, &transaction).unwrap();
        assert_eq!(store.0.len(), 2);
        apply_transaction(&mut store, &transaction.inverse("tx-2")).unwrap();
        assert!(store.0.is_empty());
    }

    #[test]
    fn workflows_validate_cycles_bind_outputs_and_execute_once() {
        let workflow = WorkflowDefinition {
            id: "wf-1".into(),
            name: "double source".into(),
            api_version: 1,
            nodes: vec![
                WorkflowNode {
                    id: "read".into(),
                    tool_id: "source".into(),
                    parameters: serde_json::json!({"value": 21}),
                    bindings: BTreeMap::new(),
                },
                WorkflowNode {
                    id: "calculate".into(),
                    tool_id: "double".into(),
                    parameters: serde_json::json!({}),
                    bindings: BTreeMap::from([("input".into(), "read.value".into())]),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "read".into(),
                to: "calculate".into(),
            }],
            metadata: BTreeMap::new(),
        };
        let run = run_workflow(
            &workflow,
            |tool| matches!(tool, "source" | "double"),
            &mut MathExecutor,
        )
        .unwrap();
        assert_eq!(run.outputs["calculate"]["value"], 42);
        let mut cyclic = workflow.clone();
        cyclic.edges.push(WorkflowEdge {
            from: "calculate".into(),
            to: "read".into(),
        });
        assert!(cyclic.execution_order(|_| true).is_err());
    }

    #[test]
    fn standards_digest_detects_tampering_and_platform_validates_trust() {
        let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let public_key = key
            .public_key()
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut package = StandardsPackage {
            id: "survey-standards".into(),
            name: "Survey standards".into(),
            version: "1.0.0".into(),
            tool_presets: BTreeMap::from([(
                "lidar.surface.dtm".into(),
                serde_json::json!({"cell_size": 1.0}),
            )]),
            validation_rules: vec![ValidationRule {
                id: "crs-required".into(),
                description: "CRS is required".into(),
                severity: ValidationSeverity::Error,
                target_kind: "project".into(),
                expression: serde_json::json!({"field": "crs", "not_null": true}),
            }],
            content_sha256: String::new(),
            signer: Some(public_key.clone()),
            signature: None,
        };
        package.seal().unwrap();
        package.signature = Some(
            key.sign(package.content_sha256.as_bytes())
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        );
        package.validate().unwrap();
        let mut state = PlatformState {
            standards: vec![package.clone()],
            trusted_signers: BTreeSet::from([public_key]),
            ..Default::default()
        };
        state.validate().unwrap();
        state.standards[0].tool_presets.clear();
        assert!(state.validate().is_err());
    }
}
