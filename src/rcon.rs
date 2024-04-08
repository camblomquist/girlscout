use std::{
    mem, str,
    sync::atomic::{AtomicI32, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{lookup_host, TcpSocket, TcpStream, ToSocketAddrs},
};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use crate::{Context, Error};

const MAX_PAYLOAD: usize = 4096;

pub struct RconClient {
    connection: TcpStream,
    req_id: AtomicI32,
}

#[repr(C, packed)]
#[derive(AsBytes, FromBytes, FromZeroes)]
struct RconPacket {
    length: i32,
    req_id: i32,
    ptype: i32,
    payload: [u8; MAX_PAYLOAD],
    _padding: [u8; 2],
}

impl RconPacket {
    fn new(req_id: i32, ptype: i32, payload: &str) -> Self {
        let length = (mem::size_of::<i32>() * 2 + payload.len() + 2) as i32;
        let pbytes = payload.as_bytes();
        let mut payload = [0; MAX_PAYLOAD];
        payload[..pbytes.len()].copy_from_slice(pbytes);
        Self {
            length,
            req_id,
            ptype,
            payload,
            _padding: [0; 2],
        }
    }

    fn payload(&self) -> &str {
        let length = self.length as usize - mem::size_of::<i32>() * 2 - 2;
        str::from_utf8(&self.payload[..length]).unwrap()
    }

    fn as_bytes(&self) -> &[u8] {
        &zerocopy::AsBytes::as_bytes(self)[..(self.length as usize + mem::size_of::<i32>())]
    }
}
impl RconClient {
    pub async fn connect<A: ToSocketAddrs>(addr: A, password: &str) -> Result<Self, Error> {
        let addr = lookup_host(addr).await?.next().unwrap();
        let socket = TcpSocket::new_v4()?;
        socket.set_keepalive(true)?;
        let connection = socket.connect(addr).await?;

        let mut client = Self {
            connection,
            req_id: AtomicI32::new(0),
        };

        client.send(3, password).await?;
        Ok(client)
    }

    async fn send(&mut self, ptype: i32, payload: &str) -> Result<String, Error> {
        let req_id = self.req_id.fetch_add(1, Ordering::Relaxed);
        let mut packet = RconPacket::new(req_id, ptype, payload);
        self.connection.write_all(packet.as_bytes()).await?;
        let _ = self.connection.read(packet.as_bytes_mut()).await?;
        if packet.req_id == req_id {
            Ok(packet.payload().into())
        } else if packet.req_id == -1 {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Unauthorized",
            )))
        } else {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Response does not match request",
            )))
        }
    }

    pub async fn send_command(&mut self, command: &str) -> Result<String, Error> {
        self.send(2, command).await
    }
}

pub async fn do_command(ctx: Context<'_>, command: String) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    let response = {
        let mut rcon = ctx
            .data()
            .rcon
            .as_ref()
            .ok_or_else(|| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "Rcon connection is unavailable",
                ))
            })?
            .lock()
            .await;
        rcon.send_command(&command).await?
    };
    let response = if !response.is_empty() {
        &response
    } else {
        "Executed command."
    };
    ctx.send(
        poise::CreateReply::default()
            .content(response)
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// Run an arbitrary server command. Response is truncated to first 4k characters
#[poise::command(slash_command, default_member_permissions = "ADMINISTRATOR")]
pub async fn command(ctx: Context<'_>, command: String) -> Result<(), Error> {
    do_command(ctx, command).await
}

#[poise::command(slash_command, default_member_permissions = "ADMINISTRATOR")]
pub async fn say(ctx: Context<'_>, message: String) -> Result<(), Error> {
    do_command(ctx, format!("say \"{message}\"")).await
}

#[poise::command(
    slash_command,
    default_member_permissions = "ADMINISTRATOR",
    subcommands("whitelist::add", "whitelist::remove"),
    subcommand_required
)]
pub async fn whitelist(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

pub mod whitelist {
    use crate::rcon::do_command;
    use crate::{Context, Error};

    /// Adds player profile(s) into the whitelist. The player does not need to be online.
    #[poise::command(slash_command)]
    pub async fn add(ctx: Context<'_>, targets: Vec<String>) -> Result<(), Error> {
        let targets = targets.join(" ");
        do_command(ctx, format!("whitelist add {targets}")).await
    }

    /// Removes player profile(s) from the whitelist. The player does not need to be online.
    #[poise::command(slash_command)]
    pub async fn remove(ctx: Context<'_>, targets: Vec<String>) -> Result<(), Error> {
        let targets = targets.join(" ");
        do_command(ctx, format!("whitelist remove {targets}")).await
    }
}
