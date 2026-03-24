use crate::settings::Settings;
use anyhow::Result;
use axum::{
    extract::State,
    response::Html,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

#[derive(Clone)]
pub struct WebState {
    pub settings: Arc<RwLock<Settings>>,
    pub settings_tx: watch::Sender<Settings>,
    pub config_path: String,
    pub sources: crate::discovery::SourceList,
}

#[derive(Serialize)]
struct SourcesResponse {
    sources: Vec<String>,
}

#[derive(Serialize)]
struct DevicesResponse {
    audio_devices: Vec<(String, String)>,
}

#[derive(Serialize)]
struct UpdateResult {
    ok: bool,
    message: String,
}

pub async fn start_web_server(port: u16, state: WebState) -> Result<()> {
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/api/config", get(get_config).post(update_config))
        .route("/api/sources", get(get_sources))
        .route("/api/devices", get(get_devices))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Web server listening on port {}", port);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn get_config(State(state): State<WebState>) -> Json<Settings> {
    let settings = state.settings.read().await;
    Json(settings.clone())
}

async fn update_config(
    State(state): State<WebState>,
    Json(new_settings): Json<Settings>,
) -> Json<UpdateResult> {
    {
        let mut settings = state.settings.write().await;
        *settings = new_settings.clone();
    }
    if let Err(e) = new_settings.save(&state.config_path) {
        return Json(UpdateResult {
            ok: false,
            message: format!("Failed to save: {}", e),
        });
    }
    let _ = state.settings_tx.send(new_settings);
    Json(UpdateResult {
        ok: true,
        message: "Saved. Changes applied.".to_string(),
    })
}

async fn get_sources(State(state): State<WebState>) -> Json<SourcesResponse> {
    let sources = state.sources.read().await;
    Json(SourcesResponse {
        sources: sources.clone(),
    })
}

async fn get_devices() -> Json<DevicesResponse> {
    #[cfg(target_os = "linux")]
    let devices = crate::audio::get_available_devices();
    #[cfg(not(target_os = "linux"))]
    let devices = vec![("Default".to_string(), "default".to_string())];

    Json(DevicesResponse {
        audio_devices: devices,
    })
}
