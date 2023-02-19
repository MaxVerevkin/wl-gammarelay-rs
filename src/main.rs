#![allow(clippy::single_component_path_imports)]

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
    let commnad = Cli::parse().command.unwrap_or(Command::Run);

    let (tx, rx) = mpsc::channel(16);

    match commnad {
        Command::Run => {
            let new_instance = dbus_server::run(tx).await?;
            if new_instance {
                wayland::run(rx).await?;
            }
        }
        Command::Watch { format } => {
            dbus_client::watch_dbus(&format).await?;
        }
    }

    Ok(())
}
