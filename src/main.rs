use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Json, State},
    response::{Html, IntoResponse, Response},
    routing::post,
    Router,
};
use config::{Config, Environment, File};
use hyper::StatusCode;
use ngrok::{config::TunnelBuilder, tunnel::HttpTunnel, Session};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, StatusCode as ReqwesStatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{signal, time::interval};
use tracing::{error, info, warn};
use tracing_subscriber;

#[derive(Debug, Clone, serde::Deserialize)]
struct Settings {
    bot_token: String,
    toggl_track_token: String,
    toggl_track_workspace_id: u64,
    ngrok_authtoken: String,
    ngrok_domain: String,
    chat_id: String,
    busy_chat_status: String,
    break_chat_status: String,
    not_working_status: String,
    minutes_till_afk: u64,
}

impl Settings {
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

// MODELS
#[derive(Debug, Deserialize)]
struct Subscription {
    subscription_id: u64,
    workspace_id: u64,
    user_id: u64,
    enabled: bool,
    description: String,
    event_filters: Vec<Value>, // Now treated as generic JSON
    url_callback: String,
    secret: String,
    validated_at: String,
    has_pending_events: bool,
    created_at: String,
    updated_at: String,
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
    Html("<h4>Ok</h4>")
}

async fn start_ngrok_listener(settings: &Settings) -> Result<HttpTunnel> {
    let session = Session::builder()
        .authtoken(&settings.ngrok_authtoken)
        .connect()
        .await?;

    let listener = session
        .http_endpoint()
        .domain(&settings.ngrok_domain)
        .listen()
        .await?;

    info!(
        "Ngrok tunnel started to listen on: {}",
        &format!("https://{}/webhook", settings.ngrok_domain)
    );

    Ok(listener)
}

async fn run_server(settings: Settings, listener: HttpTunnel) -> Result<()> {
    let last_break_start = Arc::new(AtomicU64::new(0));
    let shutdown_signal = Arc::new(tokio::sync::Notify::new());

    let app_state = AppState {
        settings: settings.clone(),
        last_break_start: last_break_start.clone(),
    };

    let router = Router::new()
        .route("/webhook", post(webhook_post).get(webhook_get))
        .with_state(app_state);

    let shutdown_signal_clone = shutdown_signal.clone();
    let shutdown_future = shutdown_signal_clone.notified();
    let server = axum::Server::builder(listener)
        .serve(router.into_make_service())
        .with_graceful_shutdown(shutdown_future);

    let ngrok_healthcheck_handler =
        tokio::spawn(ngrok_healthcheck(settings.clone(), shutdown_signal.clone()));
    let afk_status_updater_handle = tokio::spawn(afk_status_updater(
        settings.clone(),
        last_break_start.clone(),
        shutdown_signal.clone(),
    ));

    if let Err(err) = server.await {
        error!("Server error: {}", err);
    }

    shutdown_signal.notify_waiters();

    let _ = ngrok_healthcheck_handler.await;
    let _ = afk_status_updater_handle.await;

    Ok(())
}

async fn afk_status_updater(
    settings: Settings,
    last_break_start: Arc<AtomicU64>,
    shutdown_signal: Arc<tokio::sync::Notify>,
) {
    let mut interval = interval(Duration::from_secs(15));
    let client = Client::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = shutdown_signal.notified() => {
                info!("Shutting down afk_status_updater");
                break;
            }
        }

        let last_break = last_break_start.load(Ordering::Relaxed);
        if last_break == 0 {
            continue;
        }

        let current_time = get_unix_timestamp().unwrap();
        if current_time > last_break + settings.minutes_till_afk * 60 {
            let set_chat_title_url = format!(
                "https://api.telegram.org/bot{}/setChatTitle",
                settings.bot_token
            );
            let not_working_payload = json!({
                "chat_id": settings.chat_id,
                "title": settings.not_working_status
            });

            let response = client
                .post(&set_chat_title_url)
                .json(&not_working_payload)
                .send()
                .await;

            info!(
                "[SETTING NOT_WORKING] Telegram API response: {:?}",
                response
            );
            last_break_start.store(0, Ordering::Relaxed);
        }
    }
}

async fn ngrok_healthcheck(settings: Settings, shutdown_signal: Arc<tokio::sync::Notify>) {
    let client = Client::new();
    let mut interval = interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = shutdown_signal.notified() => {
                info!("Tearing down ngrok_healthcheck...");
                break;
            }
        }

        let url = format!("https://{}/webhook", settings.ngrok_domain);
        let response = client.get(&url).send().await;
        if response.is_err() || response.unwrap().status() != ReqwesStatusCode::OK {
            error!("Ngrok tunnel seems to be down. Restarting listener...");
            shutdown_signal.notify_one();
            break;
        }
    }
}

async fn ensure_toggle_track_subscription(settings: Settings) -> Result<()> {
    let client = Client::new();

    println!("SETTINGS: {:?}", settings);

    let subscriptios: Vec<Subscription> = client
        .get(&format!(
            "https://api.track.toggl.com/webhooks/api/v1/subscriptions/{}",
            settings.toggl_track_workspace_id,
        ))
        .header(CONTENT_TYPE, "application/json")
        .basic_auth(settings.toggl_track_token.clone(), Some("api_token"))
        .send()
        .await?
        .json()
        .await?;
   
    // 1. Filter subscriptions by our domain
    //
    // 2. If the length of subsctipions is zero - create the subscption 
    //
    // 3. if length of subscriptions more than 1 - delete every other in toggltrack api, get subs
    //    again and ensure that only one is left
    //
    // 4. Ensure that the one subscription is enabled
     

    println!("RESPONSE: {:?}", subscriptios);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let settings = Settings::from_config().unwrap();

    ensure_toggle_track_subscription(settings.clone()).await?;

    loop {
        let listener = match start_ngrok_listener(&settings).await {
            Ok(listener) => listener,
            Err(err) => {
                error!("Failed to start ngrok listener: {}", err);
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        let server_handler = tokio::spawn(run_server(settings.clone(), listener));

        tokio::select! {
            res = server_handler => {
                match res {
                    Ok(Ok(_)) => info!("Server exited normally."),
                    Ok(Err(err)) => error!("Server exited with error: {}", err),
                    Err(err) => error!("Server task panicked: {}", err),
                }
            }
            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down.");
                break;
            }
        }

        // Short nap before restarting
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    Ok(())
}
