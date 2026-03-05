use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Result};
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

use crate::model::TaskSpec;

#[derive(Debug, Clone)]
pub struct TaskGraph {
    graph: DiGraph<String, ()>,
    indices: BTreeMap<String, NodeIndex>,
}

impl TaskGraph {
    pub fn build(tasks: &BTreeMap<String, TaskSpec>) -> Result<Self> {
        let mut graph = DiGraph::<String, ()>::new();
        let mut indices = BTreeMap::new();

        for task_name in tasks.keys() {
            let idx = graph.add_node(task_name.clone());
            indices.insert(task_name.clone(), idx);
        }

        for (task_name, task) in tasks {
            let task_idx = *indices
                .get(task_name)
                .ok_or_else(|| anyhow!("internal graph error for task '{}'", task_name))?;

            for dep in &task.deps {
                let dep_idx = *indices.get(dep).ok_or_else(|| {
                    anyhow!("task '{}' depends on unknown task '{}'", task_name, dep)
                })?;
                graph.add_edge(dep_idx, task_idx, ());
            }
        }

        if let Err(cycle) = toposort(&graph, None) {
            let node_name = graph
                .node_weight(cycle.node_id())
                .cloned()
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(anyhow!("dependency graph contains a cycle near task '{}'", node_name));
        }

        Ok(Self { graph, indices })
    }

    pub fn all_tasks_sorted(&self) -> Vec<String> {
        self.indices.keys().cloned().collect()
    }

    pub fn required_tasks_for_target(&self, target: &str) -> Result<BTreeSet<String>> {
        let mut required = BTreeSet::new();
        let target_idx =
            *self.indices.get(target).ok_or_else(|| anyhow!("task '{}' not found", target))?;

        let mut stack = vec![target_idx];
        while let Some(node) = stack.pop() {
            let name = self
                .graph
                .node_weight(node)
                .ok_or_else(|| anyhow!("missing graph node during DFS"))?
                .clone();
            if !required.insert(name.clone()) {
                continue;
            }

            for dep_node in self.graph.neighbors_directed(node, Direction::Incoming) {
                stack.push(dep_node);
            }
        }

        Ok(required)
    }

    pub fn layers_for_target(&self, target: &str) -> Result<Vec<Vec<String>>> {
        let required = self.required_tasks_for_target(target)?;

        let mut indegree = BTreeMap::new();
        for name in &required {
            indegree.insert(name.clone(), 0usize);
        }

        for name in &required {
            let idx = self
                .indices
                .get(name)
                .copied()
                .ok_or_else(|| anyhow!("missing node index for task '{}'", name))?;

            let mut count = 0usize;
            for dep in self.graph.neighbors_directed(idx, Direction::Incoming) {
                let dep_name = self
                    .graph
                    .node_weight(dep)
                    .ok_or_else(|| anyhow!("missing node during indegree walk"))?;
                if required.contains(dep_name) {
                    count += 1;
                }
            }
            indegree.insert(name.clone(), count);
        }

        let mut ready = BTreeSet::new();
        for (name, value) in &indegree {
            if *value == 0 {
                ready.insert(name.clone());
            }
        }

        let mut scheduled = 0usize;
        let mut layers = Vec::new();

        while !ready.is_empty() {
            let layer: Vec<String> = ready.iter().cloned().collect();
            ready.clear();
            scheduled += layer.len();

            for task_name in &layer {
                let idx = self
                    .indices
                    .get(task_name)
                    .copied()
                    .ok_or_else(|| anyhow!("missing node index for task '{}'", task_name))?;

                for successor in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                    let succ_name = self
                        .graph
                        .node_weight(successor)
                        .ok_or_else(|| anyhow!("missing graph node for successor"))?
                        .clone();
                    if !required.contains(&succ_name) {
                        continue;
                    }

                    let entry = indegree.get_mut(&succ_name).ok_or_else(|| {
                        anyhow!("missing indegree entry for task '{}'", succ_name)
                    })?;
                    if *entry > 0 {
                        *entry -= 1;
                        if *entry == 0 {
                            ready.insert(succ_name);
                        }
                    }
                }
            }

            layers.push(layer);
        }

        if scheduled != required.len() {
            return Err(anyhow!(
                "dependency graph contains a cycle while creating execution layers"
            ));
        }

        Ok(layers)
    }

    pub fn dot_for_target(&self, target: &str) -> Result<String> {
        let required = self.required_tasks_for_target(target)?;
        let mut lines = vec!["digraph please {".to_string()];

        for task_name in &required {
            lines.push(format!("  \"{}\";", task_name));
        }

        for task_name in &required {
            let idx = self
                .indices
                .get(task_name)
                .copied()
                .ok_or_else(|| anyhow!("missing node index for task '{}'", task_name))?;
            for succ in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                let succ_name = self
                    .graph
                    .node_weight(succ)
                    .ok_or_else(|| anyhow!("missing graph node for dot output"))?;
                if required.contains(succ_name) {
                    lines.push(format!("  \"{}\" -> \"{}\";", task_name, succ_name));
                }
            }
        }

        lines.push("}".to_string());
        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{RunSpec, TaskSpec};

    fn task(deps: &[&str]) -> TaskSpec {
        TaskSpec {
            deps: deps.iter().map(|d| (*d).to_string()).collect(),
            inputs: vec!["src/main.rs".to_string()],
            outputs: vec!["dist/out.txt".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo ok".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        }
    }

    #[test]
    fn creates_topological_layers() {
        let mut tasks = BTreeMap::new();
        tasks.insert("a".to_string(), task(&[]));
        tasks.insert("b".to_string(), task(&["a"]));
        tasks.insert("c".to_string(), task(&["a"]));
        tasks.insert("d".to_string(), task(&["b", "c"]));

        let graph = TaskGraph::build(&tasks).expect("build graph");
        let layers = graph.layers_for_target("d").expect("build layers");

        assert_eq!(layers[0], vec!["a".to_string()]);
        assert_eq!(layers[1], vec!["b".to_string(), "c".to_string()]);
        assert_eq!(layers[2], vec!["d".to_string()]);
    }

    #[test]
    fn detects_cycles() {
        let mut tasks = BTreeMap::new();
        tasks.insert("a".to_string(), task(&["b"]));
        tasks.insert("b".to_string(), task(&["a"]));

        assert!(TaskGraph::build(&tasks).is_err());
    }
}
