use crate::{Context, Error};

#[poise::command(
    slash_command,
    subcommands("apt::moo"),
    subcommand_required,
    hide_in_help
)]
pub async fn apt(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

pub mod apt {
    use std::time;

    use crate::rcon::do_command;
    use crate::{Context, Error};

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

    #[poise::command(slash_command, hide_in_help)]
    pub async fn moo(ctx: Context<'_>) -> Result<(), Error> {
        let tagline = r#"..."Have you mooed today?"..."#;
        let i = time::UNIX_EPOCH.elapsed().unwrap().as_secs() as usize % COWS.len();
        let msg = format!("```\n{}{}\n```", COWS[i], tagline);
        ctx.say(msg).await?;
        do_command(ctx, format!("say \"{tagline}\"")).await
    }
}
