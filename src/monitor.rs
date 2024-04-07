use std::sync::Arc;

use base64::{prelude::BASE64_STANDARD, Engine};
use itertools::Itertools;
use poise::serenity_prelude::{
    json, ChannelId, Color, CreateAttachment, CreateEmbed, EditAttachments, EditMessage, Http,
    MessageId, Timestamp,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Mutex,
    time::{self, Duration},
};
use tokio_util::sync::CancellationToken;

use crate::{Context, Error};

const PROTOCOL_VERSION: u8 = 47;

fn varint_encode(mut value: i32, out: &mut [u8]) -> usize {
    const SEGMENT: i32 = 0x7F;
    const CONTINUE: i32 = 0x80;

    for (i, byte) in out.iter_mut().enumerate() {
        if (value & !SEGMENT) == 0 {
            *byte = value as u8;
            return i + 1;
        }
        *byte = ((value & SEGMENT) | CONTINUE) as u8;
        value >>= 7;
    }
    unreachable!();
}

fn varint_decode(bytes: &[u8]) -> (i32, usize) {
    let mut x: i32 = 0;
    let mut len = 5;
    for (i, byte) in bytes.iter().enumerate() {
        x |= ((byte * 0x7F) as i32) << (7 * i);
        if (byte & 0x80) == 0 {
            len = i + 1;
            break;
        }
    }
    (x, len)
}

#[derive(poise::ChoiceParameter)]
pub enum MonitorParameter {
    #[name = "status"]
    Status,
}

#[derive(Deserialize, Serialize)]
pub enum MonitorType {
    Status {
        name: String,
        host: String,
        port: u16,
        mid: MessageId,
    },
    Advancement {
        port: u16,
    },
    Death {
        port: u16,
    },
}

pub struct ServiceContext {
    services: Arc<Mutex<Vec<Arc<MonitorService>>>>,
}

impl ServiceContext {
    pub fn new(services: Arc<Mutex<Vec<Arc<MonitorService>>>>) -> Self {
        Self { services }
    }

    pub fn from_ctx(ctx: Context<'_>) -> Self {
        Self {
            services: ctx.data().services.1.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct MonitorService {
    channel_id: ChannelId,
    monitor_type: MonitorType,
    #[serde(skip)]
    http: Arc<Http>,
    #[serde(skip)]
    token: CancellationToken,
}

impl MonitorService {
    pub fn new(
        http: Arc<Http>,
        token: CancellationToken,
        channel_id: ChannelId,
        monitor_type: MonitorType,
    ) -> Self {
        Self {
            channel_id,
            monitor_type,
            http,
            token,
        }
    }

    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn cancel(&self) {
        self.token.cancel()
    }

    // A smarter me might've made a trait out of this
    pub async fn run(&self, ctx: ServiceContext) -> Result<(), Error> {
        let res = match &self.monitor_type {
            MonitorType::Status {
                name,
                host,
                port,
                mid,
            } => self.run_status(name, host, *port, *mid).await,
            _ => Ok(true),
        };

        log::info!("Service in {} finished", self.channel_id());

        let should_remove = match res {
            Ok(false) => true,
            Err(err) => {
                // Not much else we can do
                log::error!("{err}");
                true
            }
            _ => false,
        };

        if should_remove {
            let mut services = ctx.services.lock().await;
            let index = services
                .iter()
                .position(|s| s.channel_id() == self.channel_id())
                .unwrap();
            services.swap_remove(index);

            log::info!("Removed service in {}", self.channel_id());
        }
        Ok(())
    }

    async fn run_status(
        &self,
        name: &str,
        host: &str,
        port: u16,
        mid: MessageId,
    ) -> Result<bool, Error> {
        let mut handshake = Vec::with_capacity(host.len() + 5);
        let mut vibuf = [0; 5];
        let len = varint_encode(host.len() as i32, &mut vibuf);
        handshake.push(PROTOCOL_VERSION);
        handshake.extend_from_slice(&vibuf[0..len]);
        handshake.extend_from_slice(host.as_bytes());
        handshake.extend_from_slice(&port.to_be_bytes());
        handshake.push(0x01);
        let handshake = handshake;
        let cid = self.channel_id;

        let mut is_online;
        let mut version = String::from("Unknown");
        let mut description = String::default();
        let mut player_count;
        let mut player_max = 0;
        let mut player_sample;
        let mut prev_favicon = String::new();
        let mut attachments = EditAttachments::new();

        //while let Ok(mut msg) = self.http.get_message(cid, mid).await {
        loop {
            let mut msg = self.http.get_message(cid, mid).await?;

            log::info!("Updating status for {}:{}", host, port);

            if let Ok(mut stream) = TcpStream::connect((host, port)).await {
                stream.write_all(&handshake).await?;
                stream.write_all(&[0]).await?;

                stream.read_exact(&mut vibuf).await?;
                let (len, i) = varint_decode(&vibuf);
                let len = len as usize;
                let mut buf = Vec::with_capacity(len);
                buf.extend_from_slice(&vibuf[i..]);
                let start = buf.len();
                buf.resize(len, 0);
                stream.read_exact(&mut buf[start..]).await?;

                let status: json::Value = json::from_slice(&buf)?;

                version = status["version"]["name"].to_string();
                description = status["description"]["text"].to_string();

                let players = &status["players"];
                player_count = players["online"].as_u64().unwrap_or(0);
                player_max = players["max"].as_u64().unwrap_or(0);
                player_sample = players["sample"]
                    .as_array()
                    .map(|players| {
                        players
                            .iter()
                            .map(|player| player["name"].to_string())
                            .join(", ")
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let favicon = status["favicon"]
                    .as_str()
                    .unwrap_or("")
                    .split_once(',')
                    .map_or("", |d| d.1);
                attachments = if msg.attachments.is_empty() || favicon != prev_favicon {
                    prev_favicon = favicon.to_owned();
                    let bytes = BASE64_STANDARD.decode(favicon)?;
                    let attachment = CreateAttachment::bytes(bytes, "server-icon.png");
                    EditAttachments::new().add(attachment)
                } else {
                    EditAttachments::new().keep(msg.attachments[0].id)
                };

                is_online = true;
            } else {
                player_count = 0;
                player_sample = String::from("None");
                is_online = false;
            };

            let (status, color) = if is_online {
                ("ONLINE", Color::FOOYOO)
            } else {
                ("OFFLINE", Color::RED)
            };

            msg.edit(
                &self.http,
                EditMessage::new().attachments(attachments.clone()).embed(
                    CreateEmbed::new()
                        .title(name)
                        .description(&description)
                        .attachment("server-icon.png")
                        .fields([
                            ("Status", status, true),
                            ("Players", &format!("{player_count}/{player_max}"), true),
                            ("Version", &version, true),
                            ("Currently Online", &player_sample, false),
                        ])
                        .timestamp(Timestamp::now())
                        .color(color),
                ),
            )
            .await?;

            log::info!("Updated status for {}:{}", host, port);

            tokio::select! {
                _ = self.token.cancelled() => break,
                _ = time::sleep(Duration::from_secs(60)) => ()
            }
        }

        Ok(self.token.is_cancelled())
    }
}

#[poise::command(
    slash_command,
    guild_only,
    default_member_permissions = "ADMINISTRATOR",
    subcommands("sub::start", "sub::stop"),
    subcommand_required
)]
pub async fn monitor(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

pub mod sub {
    use std::io::{self, ErrorKind};
    use std::sync::Arc;

    use poise::ChoiceParameter;

    use crate::monitor::{MonitorParameter, MonitorService, MonitorType};
    use crate::{Context, Error};

    use super::ServiceContext;

    async fn start_service(ctx: Context<'_>, monitor_type: MonitorType) -> Result<(), Error> {
        let service = MonitorService::new(
            ctx.serenity_context().http.clone(),
            ctx.data().cancel_token.child_token(),
            ctx.channel_id(),
            monitor_type,
        );

        let service = Arc::new(service);
        let (tracker, services) = &ctx.data().services;
        let service_clone = service.clone();
        let sctx = ServiceContext::from_ctx(ctx);
        tracker.spawn(async move { service_clone.run(sctx).await });
        services.lock().await.push(service);

        log::info!("New service started in {}", ctx.channel_id());

        Ok(())
    }

    /// Start a monitor service in this channel
    #[poise::command(slash_command)]
    pub async fn start(
        ctx: Context<'_>,
        #[rename = "type"] monitor_type: MonitorParameter,
    ) -> Result<(), Error> {
        let channel_id = ctx.channel_id();
        if ctx
            .data()
            .services
            .1
            .lock()
            .await
            .iter()
            .any(|service| service.channel_id == channel_id)
        {
            Err(Box::new(io::Error::new(
                ErrorKind::AlreadyExists,
                "A monitor service already exists in this channel",
            )))
        } else {
            log::info!(
                "Starting new {} service in {}",
                monitor_type.name(),
                ctx.channel_id()
            );

            let monitor_type = match monitor_type {
                MonitorParameter::Status => MonitorType::Status {
                    name: ctx.data().server_name.clone(),
                    host: ctx.data().server_hostname.clone(),
                    port: ctx.data().server_port,
                    mid: ctx.channel_id().say(ctx, "Initializing service").await?.id,
                },
            };

            ctx.defer_ephemeral().await?;

            start_service(ctx, monitor_type).await?;

            ctx.say("Started service").await?;

            Ok(())
        }
    }

    #[poise::command(slash_command)]
    pub async fn stop(ctx: Context<'_>) -> Result<(), Error> {
        let channel_id = ctx.channel_id();
        let mut services = ctx.data().services.1.lock().await;
        let index = services
            .iter()
            .position(|service| service.channel_id() == channel_id);
        if let Some(index) = index {
            log::info!("Stopping services in {}...", ctx.channel_id());

            services[index].cancel();
            services.swap_remove(index);
            ctx.say("Service stopped").await?;

            log::info!("Service in {} stopped", ctx.channel_id());
            Ok(())
        } else {
            Err(Box::new(io::Error::new(
                ErrorKind::NotFound,
                "This channel is not running a service",
            )))
        }
    }
}
