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

    #[arg(long = "output-name", value_name = "OUTPUT_NAME")]
    output_names: Vec<String>,
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
    let new_instance = dbus_server::run(tx, cli.output_names).await?;

    match command {
        Command::Run => {
            if new_instance {
                wayland::run(rx).await?;
            }
        }
        Command::Watch { format } => {
            if new_instance {
                tokio::try_join!(wayland::run(rx), dbus_client::watch_dbus(&format))?;
            } else {
                dbus_client::watch_dbus(&format).await?;
            }
        }
    }

    Ok(())
}
