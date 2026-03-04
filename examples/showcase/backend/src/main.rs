use std::env;
use std::net::SocketAddr;

use axum::{
    extract::State,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    service_name: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: String,
    timestamp_utc: String,
}

#[derive(Serialize)]
struct Metric {
    name: &'static str,
    value: String,
    detail: &'static str,
}

#[derive(Serialize)]
struct MetricsResponse {
    metrics: Vec<Metric>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let static_dir = env::var("STATIC_DIR").unwrap_or_else(|_| "frontend/dist".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);

    let state = AppState { service_name: "please-showcase".to_string() };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/metrics", get(metrics))
        .with_state(state)
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{}/index.html", static_dir))),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("showcase server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        service: state.service_name,
        timestamp_utc: chrono::Utc::now().to_rfc3339(),
    })
}

async fn metrics() -> impl IntoResponse {
    Json(MetricsResponse {
        metrics: vec![
            Metric {
                name: "cache_hit_ratio",
                value: "0.93".to_string(),
                detail: "Warm reruns are mostly cache hits",
            },
            Metric {
                name: "avg_stage_time_ms",
                value: "42".to_string(),
                detail: "Copy-on-write staging in normal conditions",
            },
            Metric {
                name: "parallel_tasks",
                value: "4".to_string(),
                detail: "Concurrent DAG workers active",
            },
        ],
    })
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .try_init();
}
