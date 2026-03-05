use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use please_core::fingerprint::compute_fingerprint;
use please_core::graph::TaskGraph;
use please_core::model::{RunSpec, TaskSpec};

fn benchmark_graph_layers(c: &mut Criterion) {
    c.bench_function("graph_layers_100", |b| {
        let mut tasks = BTreeMap::new();
        tasks.insert("task_0".to_string(), task(&[]));
        for idx in 1..100 {
            let dep = format!("task_{}", idx - 1);
            tasks.insert(format!("task_{}", idx), task(&[dep.as_str()]));
        }

        let graph = TaskGraph::build(&tasks).expect("graph build");

        b.iter(|| {
            let layers = graph.layers_for_target(black_box("task_99")).expect("layers");
            black_box(layers);
        });
    });
}

fn benchmark_fingerprint(c: &mut Criterion) {
    c.bench_function("fingerprint_small_workspace", |b| {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src");

        let mut file = fs::File::create(src_dir.join("main.rs")).expect("create main.rs");
        file.write_all(b"fn main() { println!(\"hi\"); }").expect("write main.rs");

        let task = TaskSpec {
            deps: Vec::new(),
            inputs: vec!["src/main.rs".to_string()],
            outputs: vec!["dist/app".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("cargo build --release".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        };

        let resolved = vec![PathBuf::from("src/main.rs")];
        let resolved_env = BTreeMap::new();
        let secret_env = BTreeSet::new();
        let passthrough = Vec::new();

        b.iter(|| {
            let fp = compute_fingerprint(
                tmp.path(),
                "build",
                &task,
                &resolved,
                &resolved_env,
                &secret_env,
                &passthrough,
            )
            .expect("fingerprint");
            black_box(fp);
        });
    });
}

fn task(deps: &[&str]) -> TaskSpec {
    TaskSpec {
        deps: deps.iter().map(|d| d.to_string()).collect(),
        inputs: vec!["src/main.rs".to_string()],
        outputs: vec!["dist/out".to_string()],
        env: BTreeMap::new(),
        env_inherit: Vec::new(),
        secret_env: Vec::new(),
        run: RunSpec::Shell("echo ok".to_string()),
        isolation: None,
        mode: None,
        working_dir: None,
    }
}

criterion_group!(benches, benchmark_graph_layers, benchmark_fingerprint);
criterion_main!(benches);
