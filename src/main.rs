mod color;
mod dbus_client;
mod dbus_server;
mod wayland;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

#[derive(Debug, Parser)]
#[clap(author, version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server
    Run,
    /// Watch updates
    Watch { format: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Run);

    let (tx, rx) = mpsc::channel(16);
    let new_instance = dbus_server::run(tx.clone()).await?;

    match command {
        Command::Run => {
            if let Some(instance) = new_instance {
                wayland::run(rx, tx.clone(), instance).await?;
            }
        }
        Command::Watch { format } => {
            if let Some(instance) = new_instance {
                tokio::try_join!(
                    wayland::run(rx, tx.clone(), instance),
                    dbus_client::watch_dbus(&format)
                )?;
            } else {
                dbus_client::watch_dbus(&format).await?;
            }
        }
    }

    Ok(())
}
