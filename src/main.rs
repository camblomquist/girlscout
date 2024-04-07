use std::{path::PathBuf, sync::Arc};

use futures::{stream, StreamExt};
use monitor::MonitorService;
use poise::serenity_prelude::{self as serenity, ChannelId};
use rcon::RconClient;
use serde_json::Value;
use tokio::sync::Mutex;
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
    rcon: Mutex<RconClient>,
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
    let rcon_port: u16 =
        std::env::var("RCON_PORT").map_or(25575, |p| p.parse().expect("Invalid RCON_PORT"));
    let rcon_password = std::env::var("RCON_PASSWORD").expect("missing RCON_PASSWORD");
    let rcon = RconClient::connect((server_hostname.as_ref(), rcon_port), &rcon_password)
        .await
        .expect("RCON Unable to connect");
    let rcon = Mutex::new(rcon);

    let options = poise::FrameworkOptions {
        commands: vec![
            misc::apt(),
            monitor::monitor(),
            rcon::command(),
            rcon::say(),
            rcon::whitelist(),
        ],
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
                            ChannelId::new(value["channel_id"].to_string().parse().unwrap()),
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
                tokio::spawn(async move {
                    tokio::signal::ctrl_c().await.unwrap();

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
    client.unwrap().start().await.unwrap();
}
