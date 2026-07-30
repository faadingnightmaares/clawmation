//! Image references embedded in Loop node configurations.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::node_graph::NodeGraph;

pub fn image_paths(graph: &NodeGraph) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for_each_step(graph, |step| {
        if let Some(path) = step.get("template").and_then(Value::as_str) {
            push_distinct(path, &mut paths, &mut seen);
        }
        if let Some(alternatives) = step.get("templates").and_then(Value::as_array) {
            for path in alternatives.iter().filter_map(Value::as_str) {
                push_distinct(path, &mut paths, &mut seen);
            }
        }
    });
    paths
}

pub fn remap_image_paths(graph: &mut NodeGraph, remap: &HashMap<String, String>) {
    for_each_step_mut(graph, |step| {
        if let Some(path) = step.get_mut("template") {
            remap_value(path, remap);
        }
        if let Some(alternatives) = step.get_mut("templates").and_then(Value::as_array_mut) {
            for path in alternatives {
                remap_value(path, remap);
            }
        }
    });
}

fn push_distinct(path: &str, paths: &mut Vec<String>, seen: &mut HashSet<String>) {
    let path = path.trim();
    if !path.is_empty() && seen.insert(path.to_string()) {
        paths.push(path.to_string());
    }
}

fn remap_value(value: &mut Value, remap: &HashMap<String, String>) {
    let Some(path) = value.as_str() else {
        return;
    };
    if let Some(installed) = remap.get(path) {
        *value = Value::String(installed.clone());
    }
}

fn for_each_step(graph: &NodeGraph, mut visit: impl FnMut(&Value)) {
    for node in &graph.nodes {
        if let Some(step) = node.config.get("step") {
            visit(step);
        }
        if let Some(steps) = node.config.get("embedded_steps").and_then(Value::as_array) {
            for step in steps {
                visit(step);
            }
        }
    }
}

fn for_each_step_mut(graph: &mut NodeGraph, mut visit: impl FnMut(&mut Value)) {
    for node in &mut graph.nodes {
        if let Some(step) = node.config.get_mut("step") {
            visit(step);
        }
        if let Some(steps) = node
            .config
            .get_mut("embedded_steps")
            .and_then(Value::as_array_mut)
        {
            for step in steps {
                visit(step);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node_graph::GraphNode;
    use serde_json::json;

    #[test]
    fn direct_and_embedded_images_are_collected_once_and_remapped() {
        let mut graph = NodeGraph {
            nodes: vec![
                GraphNode {
                    config: json!({
                        "step": {
                            "template": "normal.png",
                            "templates": ["hover.png", "normal.png", ""]
                        }
                    }),
                    ..Default::default()
                },
                GraphNode {
                    config: json!({
                        "embedded_steps": [{
                            "template": "embedded.png",
                            "templates": ["hover.png"]
                        }]
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        assert_eq!(
            image_paths(&graph),
            vec!["normal.png", "hover.png", "embedded.png"]
        );

        remap_image_paths(
            &mut graph,
            &HashMap::from([
                ("normal.png".to_string(), "assets/a.png".to_string()),
                ("hover.png".to_string(), "assets/b.png".to_string()),
                ("embedded.png".to_string(), "assets/c.png".to_string()),
            ]),
        );
        assert_eq!(graph.nodes[0].config["step"]["template"], "assets/a.png");
        assert_eq!(
            graph.nodes[0].config["step"]["templates"],
            json!(["assets/b.png", "assets/a.png", ""])
        );
        assert_eq!(
            graph.nodes[1].config["embedded_steps"][0]["template"],
            "assets/c.png"
        );
    }
}
