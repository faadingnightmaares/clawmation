//! Persisted directed node graphs for advanced macros.
//!
//! Graphs are stored separately from recordings and legacy fine-tuned steps.
//! That keeps old macros byte-compatible while allowing branching execution.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::step::Step;

pub const GRAPH_VERSION: u32 = 1;
pub const MAX_GRAPH_NODES: usize = 1_000;
pub const MAX_GRAPH_EDGES: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

impl Default for NodePosition {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub label: String,
    pub position: NodePosition,
    pub enabled: bool,
    pub config: Value,
}

impl Default for GraphNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            node_type: String::new(),
            label: String::new(),
            position: NodePosition::default(),
            enabled: true,
            config: json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphEdge {
    pub id: String,
    pub from: String,
    pub output: String,
    pub to: String,
}

impl Default for GraphEdge {
    fn default() -> Self {
        Self {
            id: String::new(),
            from: String::new(),
            output: "next".to_string(),
            to: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeGraph {
    pub version: u32,
    pub name: String,
    pub entry: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl Default for NodeGraph {
    fn default() -> Self {
        Self {
            version: GRAPH_VERSION,
            name: String::new(),
            entry: String::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl NodeGraph {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::util::write_atomic(path, json.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn validate(&self) -> ValidationReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if self.version != GRAPH_VERSION {
            errors.push(format!("Unsupported graph version {}", self.version));
        }
        if self.nodes.is_empty() {
            errors.push("Graph has no nodes".to_string());
        }
        if self.nodes.len() > MAX_GRAPH_NODES {
            errors.push(format!("Graph exceeds {MAX_GRAPH_NODES} nodes"));
        }
        if self.edges.len() > MAX_GRAPH_EDGES {
            errors.push(format!("Graph exceeds {MAX_GRAPH_EDGES} edges"));
        }

        let mut node_ids = HashSet::new();
        for node in &self.nodes {
            if node.id.trim().is_empty() {
                errors.push("Node id cannot be empty".to_string());
            } else if !node_ids.insert(node.id.clone()) {
                errors.push(format!("Duplicate node id '{}'", node.id));
            }
            if !matches!(
                node.node_type.as_str(),
                "start" | "action" | "vision" | "branch" | "loop" | "sub_macro" | "stop"
            ) {
                errors.push(format!(
                    "Node '{}' has unknown type '{}'",
                    node.id, node.node_type
                ));
            }
            if matches!(node.node_type.as_str(), "action" | "vision")
                && serde_json::from_value::<Step>(
                    node.config.get("step").cloned().unwrap_or(Value::Null),
                )
                .is_err()
            {
                errors.push(format!("Node '{}' has an invalid step", node.id));
            }
            if node.node_type == "loop" {
                let count = node
                    .config
                    .get("count")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                if !(0..=1_000_000).contains(&count) {
                    errors.push(format!("Loop '{}' count must be 0..1000000", node.id));
                }
            }
            if node.node_type == "sub_macro"
                && node
                    .config
                    .get("macro_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                errors.push(format!("Sub-macro node '{}' needs a macro name", node.id));
            }
        }

        if self.entry.is_empty() || !node_ids.contains(&self.entry) {
            errors.push("Entry node does not exist".to_string());
        }

        let mut edge_ids = HashSet::new();
        let mut outputs = HashSet::new();
        let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in &self.edges {
            if edge.id.trim().is_empty() {
                errors.push("Edge id cannot be empty".to_string());
            } else if !edge_ids.insert(edge.id.clone()) {
                errors.push(format!("Duplicate edge id '{}'", edge.id));
            }
            if !node_ids.contains(&edge.from) {
                errors.push(format!(
                    "Edge '{}' has missing source '{}'",
                    edge.id, edge.from
                ));
            }
            if !node_ids.contains(&edge.to) {
                errors.push(format!(
                    "Edge '{}' has missing target '{}'",
                    edge.id, edge.to
                ));
            }
            if edge.output.trim().is_empty() {
                errors.push(format!("Edge '{}' has an empty output", edge.id));
            }
            if !outputs.insert((edge.from.clone(), edge.output.clone())) {
                errors.push(format!(
                    "Node '{}' has more than one '{}' output",
                    edge.from, edge.output
                ));
            }
            adjacency.entry(&edge.from).or_default().push(&edge.to);
        }

        if node_ids.contains(&self.entry) {
            let mut reachable = HashSet::new();
            let mut queue = VecDeque::from([self.entry.as_str()]);
            while let Some(id) = queue.pop_front() {
                if !reachable.insert(id) {
                    continue;
                }
                if let Some(next) = adjacency.get(id) {
                    queue.extend(next.iter().copied());
                }
            }
            for id in node_ids
                .iter()
                .filter(|id| !reachable.contains(id.as_str()))
            {
                warnings.push(format!("Node '{id}' is unreachable"));
            }
        }

        ValidationReport {
            ok: errors.is_empty(),
            errors,
            warnings,
        }
    }

    /// Seed a graph from the legacy flat list without changing the old file.
    pub fn from_steps(name: &str, steps: Vec<Step>) -> Self {
        let mut nodes = vec![GraphNode {
            id: "start".to_string(),
            node_type: "start".to_string(),
            label: "Start".to_string(),
            position: NodePosition { x: 40.0, y: 160.0 },
            ..Default::default()
        }];
        let mut edges = Vec::new();
        let mut previous = "start".to_string();

        for (index, step) in steps.into_iter().enumerate() {
            let id = format!("step-{}", index + 1);
            let node_type = if matches!(step.step_type.as_str(), "find_click" | "wait_for") {
                "vision"
            } else {
                "action"
            };
            nodes.push(GraphNode {
                id: id.clone(),
                node_type: node_type.to_string(),
                label: if step.label.is_empty() {
                    step.step_type.clone()
                } else {
                    step.label.clone()
                },
                position: NodePosition {
                    x: 300.0 + (index % 4) as f64 * 280.0,
                    y: 80.0 + (index / 4) as f64 * 190.0,
                },
                enabled: step.enabled,
                config: json!({ "step": step }),
            });
            edges.push(GraphEdge {
                id: format!("edge-{}", index + 1),
                from: previous,
                output: if index == 0 {
                    "next".to_string()
                } else {
                    let prev = &nodes[nodes.len() - 2];
                    if prev.node_type == "vision" {
                        "found".to_string()
                    } else {
                        "next".to_string()
                    }
                },
                to: id.clone(),
            });
            previous = id;
        }

        let stop_id = "stop".to_string();
        nodes.push(GraphNode {
            id: stop_id.clone(),
            node_type: "stop".to_string(),
            label: "Finish".to_string(),
            position: NodePosition {
                x: 300.0 + (nodes.len().saturating_sub(1) % 4) as f64 * 280.0,
                y: 80.0 + (nodes.len().saturating_sub(1) / 4) as f64 * 190.0,
            },
            config: json!({ "success": true }),
            ..Default::default()
        });
        edges.push(GraphEdge {
            id: "edge-finish".to_string(),
            from: previous,
            output: nodes
                .iter()
                .find(|node| node.id == edges.last().map(|e| e.to.as_str()).unwrap_or(""))
                .filter(|node| node.node_type == "vision")
                .map(|_| "found")
                .unwrap_or("next")
                .to_string(),
            to: stop_id,
        });

        Self {
            version: GRAPH_VERSION,
            name: name.to_string(),
            entry: "start".to_string(),
            nodes,
            edges,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_duplicate_outputs_and_missing_targets() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![GraphNode {
                id: "start".into(),
                node_type: "start".into(),
                ..Default::default()
            }],
            edges: vec![
                GraphEdge {
                    id: "a".into(),
                    from: "start".into(),
                    output: "next".into(),
                    to: "gone".into(),
                },
                GraphEdge {
                    id: "b".into(),
                    from: "start".into(),
                    output: "next".into(),
                    to: "start".into(),
                },
            ],
            ..Default::default()
        };
        let report = graph.validate();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("missing target")));
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("more than one 'next'")));
    }

    #[test]
    fn legacy_steps_become_a_valid_connected_graph() {
        let graph = NodeGraph::from_steps(
            "demo",
            vec![Step {
                id: "s1".into(),
                step_type: "click".into(),
                x: 10,
                y: 20,
                label: "Click".into(),
                ..Default::default()
            }],
        );
        assert!(graph.validate().ok, "{:?}", graph.validate().errors);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.nodes[1].config["step"]["x"], 10);
    }
}
