use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Json, State},
    response::{Html, IntoResponse, Response},
    routing::post,
    Router,
};
use config::{Config, File, Environment};
use hyper::StatusCode;
use ngrok::{config::TunnelBuilder, Session};
use reqwest::Client;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber;

#[derive(Debug, Clone, serde::Deserialize)]
struct Settings {
    bot_token: String,
    ngrok_authtoken: String,
    ngrok_domain: String,
    chat_id: String,
    busy_chat_status: String,
    break_chat_status: String,
    not_working_status: String,
    minutes_till_afk: u64,
}

impl Settings {
    // study anyhow
    fn from_config() -> anyhow::Result<Self> {
        let config_path = shellexpand::tilde("~/.config/amibussy/settings.yaml").to_string();
        let settings = Config::builder()
            .add_source(File::with_name(&config_path))
            // TODO: Reflect in docs
            .add_source(Environment::with_prefix("AMIBUSSY"))
            .build()?;

        let settings: Self = settings.try_deserialize()?;
        Ok(settings)
    }
}

#[derive(Clone)]
struct AppState {
    settings: Settings,
    last_break_start: Arc<AtomicU64>,
}

fn get_unix_timestamp() -> anyhow::Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

async fn webhook_post(State(state): State<AppState>, body: Bytes) -> Response {
    let request_body: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            warn!("Error parsing request body: {}", err);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    info!("GOT POST REQUEST FROM TOGGL TRACK: {}", request_body);

    let client = Client::new();

    let event_id = request_body.get("event_id");
    let event_payload = request_body.get("payload");

    if event_id.is_none() || event_payload.is_none() {
        error!(
            "Unknown event received. Breaking change in TogglTrack API? {:?}",
            request_body
        );
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }

    if let Some(Value::String(s)) = event_payload {
        if s == "ping" {
            info!("Processing ping request validation...");
            if let Some(validation_code) =
                request_body.get("validation_code").and_then(|v| v.as_str())
            {
                let response_body = json!({ "validation_code": validation_code });
                return (StatusCode::OK, Json(response_body)).into_response();
            } else {
                error!("Validation code missing in PING event");
                return StatusCode::BAD_REQUEST.into_response();
            }
        }
    }

    if let Some(Value::Object(event_payload_obj)) = event_payload {
        let start = event_payload_obj.get("start").and_then(|v| v.as_str());
        let stop = event_payload_obj.get("stop").and_then(|v| v.as_str());
        let set_chat_title_url = format!(
            "https://api.telegram.org/bot{}/setChatTitle",
            state.settings.bot_token
        );

        let bussy_payload = serde_json::json!({
                "chat_id": state.settings.chat_id,
                "title": state.settings.busy_chat_status
        });

        let break_payload = serde_json::json!({
                "chat_id": state.settings.chat_id,
                "title": state.settings.break_chat_status
        });

        if let (Some(start_time), Some(stop_time)) = (start, stop) {
            info!(
                "[SETTING BREAK]. Reason: Stop event received with payload. start_time: {}, stop_time: {}",
                start_time, stop_time
            );

            let current_time = get_unix_timestamp().unwrap();
            state
                .last_break_start
                .store(current_time, Ordering::Relaxed);

            let telegram_api_response = client
                .post(&set_chat_title_url)
                .header("Content-Type", "application/json")
                .json(&break_payload)
                .send()
                .await;

            match telegram_api_response {
                Ok(resp) if resp.status().is_success() => {
                    info!("Successfully updated chat title");
                }
                Ok(resp) => {
                    error!("Failed to update chat title, status: {}", resp.status());
                }
                Err(err) => {
                    error!("HTTP request error: {}", err);
                }
            }
            return StatusCode::OK.into_response();
        }

        if let Some(start_time) = start {
            info!(
                "[SETTING BUSY]. Reason: Start event received with payload: {}",
                start_time
            );

            let telegram_api_response = client
                .post(&set_chat_title_url)
                .header("Content-Type", "application/json")
                .json(&bussy_payload)
                .send()
                .await;

            match telegram_api_response {
                Ok(resp) if resp.status().is_success() => {
                    info!("Successfully updated chat title");
                }
                Ok(resp) => {
                    error!("Failed to update chat title, status: {}", resp.status());
                }
                Err(err) => {
                    error!("HTTP request error: {}", err);
                }
            }

            state.last_break_start.store(0, Ordering::Relaxed);
            return StatusCode::OK.into_response();
        }
    }

    return StatusCode::OK.into_response();
}

async fn webhook_get() -> Html<&'static str> {
    // just for debugging.
    Html("<h1>Nothing interesting here, stranger.</h1>")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let settings = Settings::from_config().unwrap();

    info!("RUNNING AMIBUSSY WITH SETTINGS: {:?}", settings);

    let last_break_start = Arc::new(AtomicU64::new(0));

    // TODO: Add check of the subsctiption ID. It is it not created - create it and save in the
    // config file.
    // TODO: Add check of the subsctiption status. Enable it is is disabled.

    tokio::spawn({
        let settings = settings.clone();
        let last_break_start_clone = last_break_start.clone();
        async move {
            // Sleep for a short duration to avoid busy waiting
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;

                let last_break = last_break_start_clone.load(Ordering::Relaxed);
                if last_break == 0 {
                    continue;
                }

                let current_time = get_unix_timestamp().unwrap();

                if current_time > last_break + settings.minutes_till_afk * 60 {
                    let set_chat_title_url = format!(
                        "https://api.telegram.org/bot{}/setChatTitle",
                        settings.bot_token
                    );
                    let client = Client::new();
                    let not_working_payload = serde_json::json!({
                        "chat_id": settings.chat_id,
                        "title": settings.not_working_status
                    });

                    let response = client
                        .post(&set_chat_title_url)
                        .json(&not_working_payload)
                        .send()
                        .await;

                    info!("[SETTING NOT_WORKING]. Reason: No Toggl Track timer runninng, Telegram API Response: {:?}", response);

                    last_break_start_clone.store(0, Ordering::Relaxed);
                }
            }
        }
    });

    // Ngrok keepalive.
    tokio::spawn({
        let ngrok_url = format!("https://{}/webhook", settings.ngrok_domain.clone());
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            let client = Client::new();
            loop {
                interval.tick().await;
                info!("Sending Ngrok keep-alive request to: {}", ngrok_url);
                if let Err(err) = client.get(&ngrok_url).send().await {
                    error!("Ngrok keep-alive request failed: {}", err);
                }
            }
        }
    });


    // Main server part.
    let listener = Session::builder()
        .authtoken(&settings.ngrok_authtoken)
        .connect()
        .await?
        .http_endpoint()
        .domain(&settings.ngrok_domain)
        .listen()
        .await?;

    let app_state = AppState {
        settings,
        last_break_start: last_break_start.clone(),
    };

    let toggltrack_router = Router::new()
        .route("/webhook", post(webhook_post).get(webhook_get))
        .with_state(app_state);

    let server = axum::Server::builder(listener).serve(toggltrack_router.into_make_service());

    tokio::select! {
        res = server => {
            if let Err(err) = res {
                error!("Server error: {}", err);
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down");
        }
    }

    Ok(())
}
