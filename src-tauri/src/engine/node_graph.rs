//! Hardware-free directed graph executor.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use crate::engine::ai::{self, Actuate, Detect};
use crate::models::node_graph::{GraphEdge, GraphNode, NodeGraph};
use crate::models::step::Step;

const MAX_TRANSITIONS: usize = 10_000;

pub type RunSubMacro<'a> = dyn Fn(&str, &[Step], i64) -> Result<String, String> + Send + Sync + 'a;
pub type RunChain<'a> = dyn Fn(&str) -> Result<String, String> + Send + Sync + 'a;

fn next<'a>(edges: &'a [GraphEdge], node: &str, output: &str) -> Option<&'a str> {
    edges
        .iter()
        .find(|edge| edge.from == node && edge.output == output)
        .map(|edge| edge.to.as_str())
}

fn failure_mode(graph: &NodeGraph, node: &GraphNode) -> &'static str {
    match node.config.get("failure_mode").and_then(Value::as_str) {
        Some("continue") => "continue",
        Some("recovery") => "recovery",
        Some("stop") => "stop",
        _ if next(&graph.edges, &node.id, "error").is_some() => "recovery",
        _ => "stop",
    }
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
    run_chain: &RunChain<'_>,
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
    let mut active_loops: Vec<&str> = Vec::new();

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
                    match failure_mode(graph, node) {
                        "continue" => "next",
                        "recovery" => "error",
                        _ => "__stop_failure",
                    }
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
                    if active_loops.last().copied() != Some(node.id.as_str()) {
                        active_loops.push(node.id.as_str());
                    }
                    results.push(result_row(node, true, format!("iteration {}", *count)));
                    "body"
                } else {
                    loop_counts.remove(node.id.as_str());
                    if active_loops.last().copied() == Some(node.id.as_str()) {
                        active_loops.pop();
                    }
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
                let embedded_steps = node
                    .config
                    .get("embedded_steps")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<Vec<Step>>(value).ok())
                    .unwrap_or_default();
                let repeat = node
                    .config
                    .get("repeat")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                match run_sub_macro(name, &embedded_steps, repeat) {
                    Ok(message) => {
                        last_ok = true;
                        results.push(result_row(node, true, message));
                        "success"
                    }
                    Err(message) => {
                        last_ok = false;
                        results.push(result_row(node, false, message));
                        match failure_mode(graph, node) {
                            "continue" => "success",
                            "recovery" => "error",
                            _ => "__stop_failure",
                        }
                    }
                }
            }
            "chain" => {
                let chain_id = node
                    .config
                    .get("chain_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match run_chain(chain_id) {
                    Ok(message) => {
                        last_ok = true;
                        results.push(result_row(node, true, message));
                        "success"
                    }
                    Err(message) => {
                        last_ok = false;
                        results.push(result_row(node, false, message));
                        match failure_mode(graph, node) {
                            "continue" => "success",
                            "recovery" => "error",
                            _ => "__stop_failure",
                        }
                    }
                }
            }
            "note" => {
                results.push(result_row(node, true, "note"));
                "next"
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

        if output == "__stop_failure" {
            let passed = results
                .iter()
                .filter(|result| result["ok"] == json!(true))
                .count();
            return json!({
                "ok": false,
                "error": format!("Node '{}' failed and stopped the Loop", node.id),
                "failed_node": node.id,
                "transitions": transitions,
                "nodes_run": results.len(),
                "nodes_passed": passed,
                "results": results,
            });
        }

        match next(&graph.edges, &node.id, output) {
            Some(target) => current = target,
            None if active_loops.last().is_some() => {
                current = active_loops
                    .last()
                    .copied()
                    .expect("active Loop checked above");
            }
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
            ..Default::default()
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
        let actuate: Actuate = Box::new(move |action| {
            sink.lock().unwrap().push(action);
            Ok(())
        });
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
        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );
        assert_eq!(summary["ok"], true);
        assert_eq!(*actions.lock().unwrap(), vec![Action::Click(4, 5)]);
    }

    #[test]
    fn failed_action_stops_without_a_recovery_path_by_default() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node(
                    "click",
                    "action",
                    json!({"step":{"type":"click","x":4,"y":5}}),
                ),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("start", "next", "click"),
                edge("click", "next", "stop"),
            ],
            ..Default::default()
        };
        let (detect, _, _) = seams(false);
        let actuate: Actuate = Box::new(|_| Err("input rejected".into()));
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], false);
        assert_eq!(summary["nodes_run"], 1);
        assert!(summary["error"].as_str().unwrap().contains("stopped"));
    }

    #[test]
    fn failed_action_can_continue_on_its_primary_path() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node(
                    "click",
                    "action",
                    json!({
                        "failure_mode":"continue",
                        "step":{"type":"click","x":4,"y":5}
                    }),
                ),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("start", "next", "click"),
                edge("click", "next", "stop"),
            ],
            ..Default::default()
        };
        let (detect, _, _) = seams(false);
        let actuate: Actuate = Box::new(|_| Err("input rejected".into()));
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], true);
        assert_eq!(summary["results"][0]["ok"], false);
        assert_eq!(summary["results"][1]["node_id"], "stop");
    }

    #[test]
    fn an_existing_error_edge_keeps_legacy_recovery_behavior() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node(
                    "click",
                    "action",
                    json!({"step":{"type":"click","x":4,"y":5}}),
                ),
                node("recovered", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("start", "next", "click"),
                edge("click", "error", "recovered"),
            ],
            ..Default::default()
        };
        let (detect, _, _) = seams(false);
        let actuate: Actuate = Box::new(|_| Err("input rejected".into()));
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], true);
        assert_eq!(summary["results"][1]["node_id"], "recovered");
    }

    #[test]
    fn missing_vision_uses_missing_branch() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("vision", "vision", json!({"step":{"type":"find_click"}})),
                node("good", "stop", json!({"success":true})),
                node("bad", "stop", json!({"success":false})),
            ],
            edges: vec![
                edge("start", "next", "vision"),
                edge("vision", "found", "good"),
                edge("vision", "missing", "bad"),
            ],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);
        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );
        assert_eq!(summary["ok"], false);
        assert_eq!(
            summary["results"].as_array().unwrap().last().unwrap()["node_id"],
            "bad"
        );
    }

    #[test]
    fn rejected_vision_action_uses_missing_branch_instead_of_claiming_success() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("vision", "vision", json!({"step":{"type":"find_click"}})),
                node("good", "stop", json!({"success":true})),
                node("bad", "stop", json!({"success":false})),
            ],
            edges: vec![
                edge("start", "next", "vision"),
                edge("vision", "found", "good"),
                edge("vision", "missing", "bad"),
            ],
            ..Default::default()
        };
        let (detect, _, _) = seams(true);
        let actuate: Actuate = Box::new(|_| Err("Windows rejected input".into()));
        let running = AtomicBool::new(true);
        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], false);
        assert_eq!(summary["results"][0]["ok"], false);
        assert_eq!(
            summary["results"].as_array().unwrap().last().unwrap()["node_id"],
            "bad"
        );
    }

    #[test]
    fn loop_runs_body_exact_count() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("loop", "loop", json!({"count":3})),
                node("key", "action", json!({"step":{"type":"key","key":"x"}})),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![
                edge("start", "next", "loop"),
                edge("loop", "body", "key"),
                edge("key", "next", "loop"),
                edge("loop", "done", "stop"),
            ],
            ..Default::default()
        };
        let (detect, actuate, actions) = seams(false);
        let running = AtomicBool::new(true);
        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );
        assert_eq!(summary["ok"], true);
        assert_eq!(actions.lock().unwrap().len(), 3);
    }

    #[test]
    fn forever_loop_without_a_return_edge_keeps_running_until_cancelled() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("loop", "loop", json!({"count":0})),
                node("key", "action", json!({"step":{"type":"key","key":"x"}})),
            ],
            edges: vec![edge("start", "next", "loop"), edge("loop", "body", "key")],
            ..Default::default()
        };
        let running = Arc::new(AtomicBool::new(true));
        let stop_flag = running.clone();
        let actions = Arc::new(Mutex::new(Vec::new()));
        let sink = actions.clone();
        let actuate: Actuate = Box::new(move |action| {
            let mut recorded = sink.lock().unwrap();
            recorded.push(action);
            if recorded.len() == 3 {
                stop_flag.store(false, Ordering::SeqCst);
            }
            Ok(())
        });
        let detect: Detect = Box::new(|_| (vec![], "unused".into()));

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            running.as_ref(),
        );

        assert_eq!(summary["cancelled"], true);
        assert_eq!(actions.lock().unwrap().len(), 3);
    }

    #[test]
    fn repeat_retries_a_missing_vision_result_without_a_return_edge() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("loop", "loop", json!({"count":3})),
                node(
                    "vision",
                    "vision",
                    json!({"step":{"type":"wait_for","timeout":0.0}}),
                ),
            ],
            edges: vec![
                edge("start", "next", "loop"),
                edge("loop", "body", "vision"),
            ],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], true);
        assert_eq!(
            summary["results"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|result| result["node_id"] == "vision")
                .count(),
            3
        );
    }

    #[test]
    fn loop_without_done_edge_finishes_successfully_after_its_count() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("loop", "loop", json!({"count":3})),
            ],
            edges: vec![edge("start", "next", "loop"), edge("loop", "body", "loop")],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], true);
        assert_eq!(summary["nodes_run"], 4);
    }

    #[test]
    fn a_cleared_run_flag_cancels_before_any_node_executes() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("stop", "stop", json!({"success":true})),
            ],
            edges: vec![edge("start", "next", "stop")],
            ..Default::default()
        };
        let (detect, actuate, actions) = seams(false);
        let running = AtomicBool::new(false);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("done".into()),
            &|_| Ok("done".into()),
            &running,
        );

        assert_eq!(summary["ok"], false);
        assert_eq!(summary["cancelled"], true);
        assert!(actions.lock().unwrap().is_empty());
    }

    #[test]
    fn sub_macro_node_passes_its_embedded_snapshot_and_repeat() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node(
                    "macro",
                    "sub_macro",
                    json!({
                        "macro_name":"Farm",
                        "repeat":3,
                        "embedded_steps":[{"id":"click-1","type":"click","x":4,"y":5}]
                    }),
                ),
                node("good", "stop", json!({"success":true})),
                node("bad", "stop", json!({"success":false})),
            ],
            edges: vec![
                edge("start", "next", "macro"),
                edge("macro", "success", "good"),
                edge("macro", "error", "bad"),
            ],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);
        let received = Arc::new(Mutex::new(None));
        let received_by_runner = received.clone();

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &move |name, steps, repeat| {
                *received_by_runner.lock().unwrap() =
                    Some((name.to_string(), steps.to_vec(), repeat));
                Ok("embedded macro finished".into())
            },
            &|_| Ok("chain".into()),
            &running,
        );

        assert_eq!(summary["ok"], true);
        let received = received.lock().unwrap();
        let (name, steps, repeat) = received.as_ref().unwrap();
        assert_eq!(name, "Farm");
        assert_eq!(*repeat, 3);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_type, "click");
    }

    #[test]
    fn chain_node_routes_success_and_failure() {
        let graph = NodeGraph {
            entry: "start".into(),
            nodes: vec![
                node("start", "start", json!({})),
                node("chain", "chain", json!({"chain_id":"daily"})),
                node("good", "stop", json!({"success":true})),
                node("bad", "stop", json!({"success":false})),
            ],
            edges: vec![
                edge("start", "next", "chain"),
                edge("chain", "success", "good"),
                edge("chain", "error", "bad"),
            ],
            ..Default::default()
        };
        let (detect, actuate, _) = seams(false);
        let running = AtomicBool::new(true);

        let summary = run(
            &graph,
            &detect,
            &actuate,
            &|_, _, _| Ok("macro".into()),
            &|id| {
                assert_eq!(id, "daily");
                Ok("chain finished".into())
            },
            &running,
        );

        assert_eq!(summary["ok"], true);
        assert_eq!(summary["results"][0]["message"], "chain finished");
    }
}
