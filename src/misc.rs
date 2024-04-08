use crate::{Context, Error};
use std::time;

/// a high-level commandline interface for the package management system.
#[poise::command(slash_command, hide_in_help)]
pub async fn apt(ctx: Context<'_>, arguments: String) -> Result<(), Error> {
    let command = arguments.split_whitespace().next().unwrap_or("");
    match command {
        "moo" => {
            let i = time::UNIX_EPOCH.elapsed().unwrap().as_secs() as usize % COWS.len();
            let msg = format!("```\n{}...\"Have you mooed today?\"...\n```", COWS[i]);
            ctx.say(msg).await?;

            // Ignore errors
            if let Some(rcon) = ctx.data().rcon.as_ref() {
                let mut rcon = rcon.lock().await;

                let _ = rcon
                    .send_command(&format!(
                        r#"/tellraw @a {{"text":"...\"Have you mooed today?\"..."}}"#
                    ))
                    .await;
            }
        }
        _ => {
            ctx.say(format!("Unknown command {command}")).await?;
        }
    }

    Ok(())
}

const COWS: &[&str] = &[
    concat!(
        "         (__) \n",
        "         (oo) \n",
        "   /------\\/ \n",
        "  / |    ||   \n",
        " *  /\\---/\\ \n",
        "    ~~   ~~   \n",
    ),
    concat!(
        "         (__)  \n",
        " _______~(..)~ \n",
        "   ,----\\(oo) \n",
        "  /|____|,'    \n",
        " * /\"\\ /\\   \n",
        "   ~ ~ ~ ~     \n",
    ),
    concat!(
        "                    \\_/  \n",
        "  m00h  (__)       -(_)-  \n",
        "     \\  ~Oo~___     / \\ \n",
        "        (..)  |\\         \n",
        " _________|_|_|__________ \n",
    ),
];
