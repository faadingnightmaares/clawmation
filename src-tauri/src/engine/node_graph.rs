//! Hardware-free directed graph executor.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use crate::engine::ai::{self, Actuate, Detect};
use crate::models::node_graph::{GraphEdge, GraphNode, NodeGraph};
use crate::models::step::Step;

const MAX_TRANSITIONS: usize = 10_000;

pub type RunSubMacro<'a> = dyn Fn(&str) -> Result<String, String> + Send + Sync + 'a;

fn next<'a>(edges: &'a [GraphEdge], node: &str, output: &str) -> Option<&'a str> {
    edges
        .iter()
        .find(|edge| edge.from == node && edge.output == output)
        .map(|edge| edge.to.as_str())
}

fn result_row(node: &GraphNode, ok: bool, message: impl Into<String>) -> Value {
    json!({
        "node_id": node.id,
        "label": if node.label.is_empty() { &node.node_type } else { &node.label },
        "ok": ok,
        "message": message.into(),
    })
}

pub fn run(
    graph: &NodeGraph,
    detect: &Detect,
    actuate: &Actuate,
    run_sub_macro: &RunSubMacro<'_>,
    running: &AtomicBool,
) -> Value {
    let report = graph.validate();
    if !report.ok {
        return json!({ "ok": false, "error": report.errors.join("; "), "results": [] });
    }

    let nodes: HashMap<&str, &GraphNode> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut current = graph.entry.as_str();
    let mut transitions = 0_usize;
    let mut results = Vec::new();
    let mut last_ok = true;
    let mut loop_counts: HashMap<&str, i64> = HashMap::new();

    while running.load(Ordering::Relaxed) {
        transitions += 1;
        if transitions > MAX_TRANSITIONS {
            return json!({
                "ok": false,
                "error": "Graph exceeded the 10000 transition safety limit",
                "transitions": transitions - 1,
                "results": results,
            });
        }

        let Some(node) = nodes.get(current).copied() else {
            return json!({ "ok": false, "error": format!("Missing node '{current}'"), "results": results });
        };

        if !node.enabled {
            let Some(target) = next(&graph.edges, &node.id, "next") else {
                break;
            };
            current = target;
            continue;
        }

        let output = match node.node_type.as_str() {
            "start" => "next",
            "action" | "vision" => {
                let step: Step = match serde_json::from_value(
                    node.config.get("step").cloned().unwrap_or(Value::Null),
                ) {
                    Ok(step) => step,
                    Err(error) => {
                        return json!({
                            "ok": false,
                            "error": format!("Node '{}' has a bad step: {error}", node.id),
                            "results": results,
                        })
                    }
                };
                let result = ai::execute_step(&step, detect, actuate, running);
                last_ok = result.ok;
                results.push(json!({
                    "node_id": node.id,
                    "label": if node.label.is_empty() { &step.step_type } else { &node.label },
                    "ok": result.ok,
                    "message": result.message,
                    "found_x": result.found_x,
                    "found_y": result.found_y,
                    "matched": result.matched,
                    "confidence": result.confidence,
                    "elapsed": result.elapsed,
                }));
                if node.node_type == "vision" {
                    if result.ok {
                        "found"
                    } else {
                        "missing"
                    }
                } else if result.ok {
                    "next"
                } else {
                    "error"
                }
            }
            "branch" => {
                let condition = node
                    .config
                    .get("condition")
                    .and_then(Value::as_str)
                    .unwrap_or("last_ok");
                let passes = match condition {
                    "always" => true,
                    "never" => false,
                    "last_failed" => !last_ok,
                    _ => last_ok,
                };
                results.push(result_row(
                    node,
                    true,
                    format!("condition {condition} = {passes}"),
                ));
                if passes {
                    "true"
                } else {
                    "false"
                }
            }
            "loop" => {
                let limit = node
                    .config
                    .get("count")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                let count = loop_counts.entry(&node.id).or_insert(0);
                if limit == 0 || *count < limit {
                    *count += 1;
                    results.push(result_row(node, true, format!("iteration {}", *count)));
                    "body"
                } else {
                    loop_counts.remove(node.id.as_str());
                    results.push(result_row(node, true, "loop complete"));
                    "done"
                }
            }
            "sub_macro" => {
                let name = node
                    .config
                    .get("macro_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match run_sub_macro(name) {
                    Ok(message) => {
                        last_ok = true;
                        results.push(result_row(node, true, message));
                        "success"
                    }
                    Err(message) => {
                        last_ok = false;
                        results.push(result_row(node, false, message));
                        "error"
                    }
                }
            }
            "stop" => {
                let success = node
                    .config
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                results.push(result_row(
                    node,
                    success,
                    if success { "finished" } else { "stopped" },
                ));
                let passed = results
                    .iter()
                    .filter(|result| result["ok"] == json!(true))
                    .count();
                return json!({
                    "ok": success,
                    "transitions": transitions,
                    "nodes_run": results.len(),
                    "nodes_passed": passed,
                    "results": results,
                });
            }
            other => {
                return json!({
                    "ok": false,
                    "error": format!("Unknown node type '{other}'"),
                    "results": results,
                });
            }
        };

        if !running.load(Ordering::SeqCst) {
            return json!({
                "ok": false,
                "cancelled": true,
                "error": "Stopped",
                "transitions": transitions,
                "nodes_run": results.len(),
                "results": results,
            });
        }

        match next(&graph.edges, &node.id, output) {
            Some(target) => current = target,
            None => {
                let failed = matches!(output, "error" | "missing");
                let passed = results
                    .iter()
                    .filter(|result| result["ok"] == json!(true))
                    .count();
                return json!({
                    "ok": !failed,
                    "error": if failed { format!("Node '{}' has no '{}' path", node.id, output) } else { String::new() },
                    "transitions": transitions,
                    "nodes_run": results.len(),
                    "nodes_passed": passed,
                    "results": results,
                });
            }
        }
    }

    json!({
        "ok": false,
        "cancelled": true,
        "error": "Stopped",
        "transitions": transitions,
        "nodes_run": results.len(),
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::engine::ai::{Action, Match};
    use crate::models::node_graph::{GraphEdge, GraphNode, NodePosition};

    fn node(id: &str, node_type: &str, config: Value) -> GraphNode {
        GraphNode {
            id: id.into(),
            node_type: node_type.into(),
            label: id.into(),
            position: NodePosition::default(),
            config,
            ..Default::default()
        }
    }

    fn edge(from: &str, output: &str, to: &str) -> GraphEdge {
        GraphEdge {
            id: format!("{from}-{output}-{to}"),
            from: from.into(),
            output: output.into(),
            to: to.into(),
        }
    }

    fn seams(found: bool) -> (Detect, Actuate, Arc<Mutex<Vec<Action>>>) {
        let detect: Detect = Box::new(move |_| {
            if found {
                (
                    vec![Match {
                        x: 7,
                        y: 8,
                        confidence: 0.9,
                    }],
                    "found".into(),
                )
            } else {
                (vec![], "missing".into())
            }
        });
        let actions = Arc::new(Mutex::new(Vec::new()));
        let sink = actions.clone();
        let actuate: Actuate = Box::new(move |action| sink.lock().unwrap().push(action));
        (detect, actuate, actions)
    }

    #[test]
    fn action_graph_executes_and_reaches_success_stop() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node(
                    "click",
                    "action",
                    json!({"step": {"type":"click","x":4,"y":5}}),
                ),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("start", "next", "click"),
                edge("click", "next", "stop"),
            ],
            ..Default::default()
        };
        let (detect, actuate, actions) = seams(false);
        let running = AtomicBool::new(true);
        let summary = run(&graph, &detect, &actuate, &|_| Ok("done".into()), &running);
        assert_eq!(summary["ok"], true);
        assert_eq!(*actions.lock().unwrap(), vec![Action::Click(4, 5)]);
    }

    #[test]
    fn missing_vision_uses_missing_branch() {
        let graph = NodeGraph {
            entry: "vision".into(),
            nodes: vec![
                node("vision", "vision", json!({"step":{"type":"find_click"}})),
                node("good", "stop", json!({"success":true})),
                node("bad", "stop", json!({"success":false})),
            ],
            edges: vec![
                edge("vision", "found", "good"),
                edge("vision", "missing", "bad"),
            ],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);
        let summary = run(&graph, &detect, &actuate, &|_| Ok("done".into()), &running);
        assert_eq!(summary["ok"], false);
        assert_eq!(
            summary["results"].as_array().unwrap().last().unwrap()["node_id"],
            "bad"
        );
    }

    #[test]
    fn loop_runs_body_exact_count() {
        let graph = NodeGraph {
            entry: "loop".into(),
            nodes: vec![
                node("loop", "loop", json!({"count":3})),
                node("key", "action", json!({"step":{"type":"key","key":"x"}})),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("loop", "body", "key"),
                edge("key", "next", "loop"),
                edge("loop", "done", "stop"),
            ],
            ..Default::default()
        };
        let (detect, actuate, actions) = seams(false);
        let running = AtomicBool::new(true);
        let summary = run(&graph, &detect, &actuate, &|_| Ok("done".into()), &running);
        assert_eq!(summary["ok"], true);
        assert_eq!(actions.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_cleared_run_flag_cancels_before_any_node_executes() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![node("start", "start", json!({}))],
            ..Default::default()
        };
        let (detect, actuate, actions) = seams(false);
        let running = AtomicBool::new(false);

        let summary = run(&graph, &detect, &actuate, &|_| Ok("done".into()), &running);

        assert_eq!(summary["ok"], false);
        assert_eq!(summary["cancelled"], true);
        assert!(actions.lock().unwrap().is_empty());
    }
}
