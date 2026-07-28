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
pub const MAX_EMBEDDED_STEPS: usize = 50_000;

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

fn allowed_outputs(node_type: &str) -> &'static [&'static str] {
    match node_type {
        "start" => &["next"],
        "action" => &["next", "error"],
        "vision" => &["found", "missing"],
        "branch" => &["true", "false"],
        "loop" => &["body", "done"],
        "sub_macro" | "chain" => &["success", "error"],
        "note" | "stop" => &[],
        _ => &[],
    }
}

fn required_outputs(node_type: &str) -> &'static [&'static str] {
    match node_type {
        "start" => &["next"],
        "branch" => &["true", "false"],
        "loop" => &["body", "done"],
        _ => &[],
    }
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
                "start" | "action" | "vision" | "branch" | "loop" | "sub_macro" | "chain" | "note" | "stop"
            ) {
                errors.push(format!(
                    "Node '{}' has unknown type '{}'",
                    node.id, node.node_type
                ));
            }
            if matches!(node.node_type.as_str(), "action" | "vision") {
                match serde_json::from_value::<Step>(
                    node.config.get("step").cloned().unwrap_or(Value::Null),
                ) {
                    Ok(step) => {
                        if step.step_type == "key" && step.key.trim().is_empty() {
                            errors.push(format!("Node '{}' needs a key", node.id));
                        }
                        if step.step_type == "delay" && step.delay < 0.0 {
                            errors.push(format!("Node '{}' wait cannot be negative", node.id));
                        }
                        if step.step_type == "wait_for" && step.timeout < 0.0 {
                            errors.push(format!("Node '{}' timeout cannot be negative", node.id));
                        }
                        if node.node_type == "vision"
                            && step.detect_mode == "template"
                            && step.template.trim().is_empty()
                        {
                            errors.push(format!("Vision node '{}' needs an image", node.id));
                        }
                        if node.node_type == "vision" && !(0.1..=1.0).contains(&step.confidence) {
                            errors.push(format!(
                                "Vision node '{}' confidence must be 0.1..1",
                                node.id
                            ));
                        }
                    }
                    Err(_) => errors.push(format!("Node '{}' has an invalid step", node.id)),
                }
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
            if node.node_type == "sub_macro" {
                let name = node
                    .config
                    .get("macro_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if name.is_empty() {
                    errors.push(format!("Sub-macro node '{}' needs a macro name", node.id));
                }
                let repeat = node
                    .config
                    .get("repeat")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                if !(1..=1000).contains(&repeat) {
                    errors.push(format!(
                        "Sub-macro node '{}' repeat must be 1..1000",
                        node.id
                    ));
                }
                if let Some(value) = node.config.get("embedded_steps") {
                    match serde_json::from_value::<Vec<Step>>(value.clone()) {
                        Ok(steps) => {
                            if steps.len() > MAX_EMBEDDED_STEPS {
                                errors.push(format!(
                                    "Sub-macro node '{}' exceeds {MAX_EMBEDDED_STEPS} embedded steps",
                                    node.id
                                ));
                            }
                            if steps.iter().any(|step| {
                                !matches!(
                                    step.step_type.as_str(),
                                    "click"
                                        | "key"
                                        | "type"
                                        | "scroll"
                                        | "delay"
                                        | "find_click"
                                        | "wait_for"
                                )
                            }) {
                                errors.push(format!(
                                    "Sub-macro node '{}' contains an invalid embedded step",
                                    node.id
                                ));
                            }
                        }
                        Err(_) => errors.push(format!(
                            "Sub-macro node '{}' has invalid embedded steps",
                            node.id
                        )),
                    }
                }
            }
            if node.node_type == "chain"
                && node
                    .config
                    .get("chain_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                errors.push(format!("Chain node '{}' needs a chain", node.id));
            }
        }

        let start_nodes: Vec<&GraphNode> = self
            .nodes
            .iter()
            .filter(|node| node.node_type == "start")
            .collect();
        if start_nodes.len() != 1 {
            errors.push("Graph needs exactly one Start node".to_string());
        }
        if self.entry.is_empty() || !node_ids.contains(&self.entry) {
            errors.push("Entry node does not exist".to_string());
        } else if start_nodes.len() != 1 || start_nodes[0].id != self.entry {
            errors.push("Graph entry must be the Start node".to_string());
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
            } else if let Some(source) = self.nodes.iter().find(|node| node.id == edge.from) {
                if !allowed_outputs(&source.node_type).contains(&edge.output.as_str()) {
                    errors.push(format!(
                        "Node '{}' ({}) does not have output '{}'",
                        source.id, source.node_type, edge.output
                    ));
                }
            }
            if !outputs.insert((edge.from.clone(), edge.output.clone())) {
                errors.push(format!(
                    "Node '{}' has more than one '{}' output",
                    edge.from, edge.output
                ));
            }
            adjacency.entry(&edge.from).or_default().push(&edge.to);
        }

        for node in &self.nodes {
            for output in required_outputs(&node.node_type) {
                if !outputs.contains(&(node.id.clone(), (*output).to_string())) {
                    errors.push(format!("Node '{}' needs a '{}' output", node.id, output));
                }
            }
            if !matches!(node.node_type.as_str(), "note" | "stop")
                && required_outputs(&node.node_type).is_empty()
                && allowed_outputs(&node.node_type)
                    .iter()
                    .all(|output| !outputs.contains(&(node.id.clone(), (*output).to_string())))
            {
                warnings.push(format!("Node '{}' has no connected output", node.id));
            }
        }

        let note_ids: HashSet<&str> = self
            .nodes
            .iter()
            .filter(|node| node.node_type == "note")
            .map(|node| node.id.as_str())
            .collect();
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
                .filter(|id| !reachable.contains(id.as_str()) && !note_ids.contains(id.as_str()))
            {
                warnings.push(format!("Node '{id}' is unreachable"));
            }
        }

        let node_types: HashMap<&str, &str> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.node_type.as_str()))
            .collect();
        let mut visited = HashSet::new();
        let mut stack = Vec::new();
        let mut in_stack = HashSet::new();
        let mut unsafe_cycle = false;
        fn visit<'a>(
            id: &'a str,
            adjacency: &HashMap<&'a str, Vec<&'a str>>,
            node_types: &HashMap<&'a str, &'a str>,
            visited: &mut HashSet<&'a str>,
            stack: &mut Vec<&'a str>,
            in_stack: &mut HashSet<&'a str>,
            unsafe_cycle: &mut bool,
        ) {
            if *unsafe_cycle || !visited.insert(id) {
                return;
            }
            stack.push(id);
            in_stack.insert(id);
            if let Some(next_nodes) = adjacency.get(id) {
                for next_id in next_nodes {
                    if in_stack.contains(next_id) {
                        let start = stack
                            .iter()
                            .position(|candidate| candidate == next_id)
                            .unwrap_or(0);
                        if !stack[start..]
                            .iter()
                            .any(|cycle_id| node_types.get(cycle_id).copied() == Some("loop"))
                        {
                            *unsafe_cycle = true;
                            break;
                        }
                    } else if !visited.contains(next_id) {
                        visit(
                            next_id,
                            adjacency,
                            node_types,
                            visited,
                            stack,
                            in_stack,
                            unsafe_cycle,
                        );
                    }
                }
            }
            stack.pop();
            in_stack.remove(id);
        }
        for id in node_ids.iter().map(String::as_str) {
            if !visited.contains(id) {
                visit(
                    id,
                    &adjacency,
                    &node_types,
                    &mut visited,
                    &mut stack,
                    &mut in_stack,
                    &mut unsafe_cycle,
                );
            }
        }
        if unsafe_cycle {
            errors
                .push("Graph contains a cycle that does not pass through a Loop node".to_string());
        }

        ValidationReport {
            ok: errors.is_empty(),
            errors,
            warnings,
        }
    }

    pub fn validate_with_resources(
        &self,
        macro_names: &HashSet<String>,
        chain_ids: &HashSet<String>,
    ) -> ValidationReport {
        let mut report = self.validate();
        for node in &self.nodes {
            if node.node_type == "sub_macro" {
                let name = node
                    .config
                    .get("macro_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let has_embedded_steps = node
                    .config
                    .get("embedded_steps")
                    .and_then(Value::as_array)
                    .is_some_and(|steps| !steps.is_empty());
                if has_embedded_steps {
                    continue;
                }
                if !name.is_empty() && !macro_names.contains(name) {
                    report.errors.push(format!(
                        "Sub-macro node '{}' references unknown macro '{}'",
                        node.id, name
                    ));
                }
            } else if node.node_type == "chain" {
                let chain_id = node
                    .config
                    .get("chain_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if !chain_id.is_empty() && !chain_ids.contains(chain_id) {
                    report.errors.push(format!(
                        "Chain node '{}' references unknown chain '{}'",
                        node.id, chain_id
                    ));
                }
            }
        }
        report.ok = report.errors.is_empty();
        report
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
    fn validation_requires_one_start_node_as_the_entry() {
        let graph = NodeGraph {
            entry: "stop".into(),
            nodes: vec![GraphNode {
                id: "stop".into(),
                node_type: "stop".into(),
                config: json!({"success":true}),
                ..Default::default()
            }],
            ..Default::default()
        };

        let report = graph.validate();

        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("exactly one Start node")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("entry must be the Start node")));
    }

    #[test]
    fn notes_are_valid_disconnected_canvas_annotations() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                GraphNode {
                    id: "start".into(),
                    node_type: "start".into(),
                    ..Default::default()
                },
                GraphNode {
                    id: "stop".into(),
                    node_type: "stop".into(),
                    config: json!({"success": true}),
                    ..Default::default()
                },
                GraphNode {
                    id: "note".into(),
                    node_type: "note".into(),
                    config: json!({"text": "Reconnect before continuing"}),
                    ..Default::default()
                },
            ],
            edges: vec![GraphEdge {
                id: "finish".into(),
                from: "start".into(),
                output: "next".into(),
                to: "stop".into(),
            }],
            ..Default::default()
        };

        let report = graph.validate();

        assert!(report.ok, "{:?}", report.errors);
        assert!(report.warnings.iter().all(|warning| !warning.contains("note")));
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

    #[test]
    fn validation_rejects_invalid_ports_and_cycles_without_a_loop() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                GraphNode {
                    id: "start".into(),
                    node_type: "start".into(),
                    ..Default::default()
                },
                GraphNode {
                    id: "branch".into(),
                    node_type: "branch".into(),
                    config: json!({"condition":"last_ok"}),
                    ..Default::default()
                },
            ],
            edges: vec![
                GraphEdge {
                    id: "a".into(),
                    from: "start".into(),
                    output: "error".into(),
                    to: "branch".into(),
                },
                GraphEdge {
                    id: "b".into(),
                    from: "branch".into(),
                    output: "true".into(),
                    to: "start".into(),
                },
            ],
            ..Default::default()
        };

        let report = graph.validate();

        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("output 'error'")));
        assert!(report.errors.iter().any(|e| e.contains("cycle")));
    }

    #[test]
    fn validation_checks_macro_and_chain_references() {
        let graph = NodeGraph {
            name: "current".into(),
            entry: "macro".into(),
            nodes: vec![
                GraphNode {
                    id: "macro".into(),
                    node_type: "sub_macro".into(),
                    config: json!({"macro_name":"current"}),
                    ..Default::default()
                },
                GraphNode {
                    id: "chain".into(),
                    node_type: "chain".into(),
                    config: json!({"chain_id":"missing-chain"}),
                    ..Default::default()
                },
            ],
            edges: vec![GraphEdge {
                id: "a".into(),
                from: "macro".into(),
                output: "success".into(),
                to: "chain".into(),
            }],
            ..Default::default()
        };
        let macros = HashSet::from(["current".to_string()]);
        let chains = HashSet::new();

        let report = graph.validate_with_resources(&macros, &chains);

        assert!(!report.ok);
        assert!(!report
            .errors
            .iter()
            .any(|e| e.contains("cannot call itself")));
        assert!(report.errors.iter().any(|e| e.contains("unknown chain")));
    }

    #[test]
    fn validation_requires_an_image_for_template_vision_nodes() {
        let graph = NodeGraph {
            entry: "vision".into(),
            nodes: vec![GraphNode {
                id: "vision".into(),
                node_type: "vision".into(),
                config: json!({
                    "step": {
                        "type": "wait_for",
                        "detect_mode": "template",
                        "template": ""
                    }
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let report = graph.validate();

        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("needs an image")));
    }

    #[test]
    fn embedded_macro_nodes_do_not_depend_on_the_source_macro() {
        let graph = NodeGraph {
            entry: "macro".into(),
            nodes: vec![GraphNode {
                id: "macro".into(),
                node_type: "sub_macro".into(),
                config: json!({
                    "macro_name": "deleted-source",
                    "repeat": 2,
                    "embedded_steps": [{
                        "id": "click-1",
                        "type": "click",
                        "x": 10,
                        "y": 20
                    }]
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let report = graph.validate_with_resources(&HashSet::new(), &HashSet::new());

        assert!(!report
            .errors
            .iter()
            .any(|error| error.contains("unknown macro")));
    }

    #[test]
    fn embedded_macro_repeat_must_be_bounded() {
        let graph = NodeGraph {
            entry: "macro".into(),
            nodes: vec![GraphNode {
                id: "macro".into(),
                node_type: "sub_macro".into(),
                config: json!({
                    "macro_name": "Farm",
                    "repeat": 0,
                    "embedded_steps": [{"id": "click-1", "type": "click"}]
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let report = graph.validate();

        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("repeat must be 1..1000")));
    }
}
