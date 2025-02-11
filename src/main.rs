use std::{path::PathBuf, sync::Arc};

use futures::{stream, StreamExt};
use monitor::MonitorService;
use poise::serenity_prelude::{self as serenity, ChannelId};
use rcon::RconClient;
use serde_json::Value;
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::Mutex,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::monitor::ServiceContext;

mod misc;
mod monitor;
mod rcon;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

const DEFAULT_DATA_PATH: &str = "/data";

pub struct Data {
    server_name: String,
    server_hostname: String,
    server_port: u16,
    rcon: Option<Mutex<RconClient>>,
    services: (TaskTracker, Arc<Mutex<Vec<Arc<MonitorService>>>>),
    cancel_token: CancellationToken,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();

    let data_path = std::env::var("DATA_PATH").unwrap_or(DEFAULT_DATA_PATH.to_string());
    let data_path = PathBuf::from(data_path);

    let token = std::env::var("DISCORD_TOKEN").expect("missing DISCORD_TOKEN");
    let server_name = std::env::var("SERVER_NAME").unwrap_or_else(|_| "Minecraft Server".into());
    let server_hostname = std::env::var("SERVER_HOST").unwrap_or_else(|_| "localhost".into());
    let server_port: u16 =
        std::env::var("SEVER_PORT").map_or(25565, |p| p.parse().expect("Invalid SERVER_PORT"));

    let mut commands = vec![monitor::monitor(), misc::apt()];

    let rcon = if let Ok(rcon_password) = std::env::var("RCON_PASSWORD") {
        let rcon_port: u16 =
            std::env::var("RCON_PORT").map_or(25575, |p| p.parse().expect("Invalid RCON_PORT"));

        let rcon = RconClient::connect((server_hostname.as_ref(), rcon_port), &rcon_password).await;
        match rcon {
            Ok(rcon) => {
                commands.extend([rcon::command(), rcon::say(), rcon::whitelist()]);
                Some(Mutex::new(rcon))
            }
            Err(err) => {
                log::warn!(
                    "Unable to connect to rcon (Error: {}) Commands using rcon will be unavailable",
                    err
                );
                None
            }
        }
    } else {
        log::warn!("No RCON_PASSWORD provided. Commands using rcon will be unavailable");
        None
    };

    let options = poise::FrameworkOptions {
        commands,
        ..Default::default()
    };

    let framework = poise::Framework::builder()
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                log::info!("Logged in as {}", ready.user.name);

                log::info!("Registering commands...");

                for guild in &ready.guilds {
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        guild.id,
                    )
                    .await?;
                }

                log::info!(
                    "Registered {} commands in {} guilds",
                    framework.options().commands.len(),
                    ready.guilds.len()
                );

                let tracker = TaskTracker::new();
                let cancel_token = CancellationToken::new();

                let services = tokio::fs::read(data_path.join("services.json"))
                    .await
                    .unwrap_or_else(|_| b"[]".into());
                let services: Vec<Value> = serde_json::from_slice(&services)?;
                let services = stream::iter(services)
                    .map(|value| {
                        Arc::new(MonitorService::new(
                            ctx.http.clone(),
                            cancel_token.child_token(),
                            serde_json::from_value(value["channel_id"].clone()).unwrap(),
                            serde_json::from_value(value["monitor_type"].clone()).unwrap(),
                        ))
                    })
                    .collect::<Vec<_>>()
                    .await;

                log::info!("Starting services...");

                let service_count = services.len();
                let services = Arc::new(Mutex::new(services));
                for service in &*services.lock().await {
                    let services = services.clone();
                    let service = service.clone();
                    let ctx = ServiceContext::new(services.clone());
                    tracker.spawn(async move { service.run(ctx).await });
                }

                log::info!("Started {} services", service_count);

                let services_clone = services.clone();
                let token = cancel_token.clone();
                let shard_manager = framework.shard_manager().clone();
                tokio::spawn(async move {
                    let mut signal = signal(SignalKind::terminate()).unwrap();
                    signal.recv().await.unwrap();

                    log::info!("Stopping client...");

                    shard_manager.shutdown_all().await;

                    log::info!("Stopping services...");

                    token.cancel();
                    let services = services_clone.lock().await;
                    let data = serde_json::to_string(&*services).unwrap();
                    tokio::fs::write(data_path.join("services.json"), data.as_bytes())
                        .await
                        .expect("Failed to serialize services");

                    log::info!("Stopped {} services", services.len());
                });

                let services = (tracker, services);

                Ok(Data {
                    server_name,
                    server_hostname,
                    server_port,
                    services,
                    rcon,
                    cancel_token,
                })
            })
        })
        .options(options)
        .build();
    let intents = serenity::GatewayIntents::non_privileged();
    let client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await;

    log::info!("Starting client...");

    client.unwrap().start().await.unwrap();

    log::info!("Client stopped");
}
