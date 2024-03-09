mod color;
mod dbus_client;
mod dbus_server;
mod wayland;

use std::io;
use std::os::fd::AsRawFd;

use clap::{Parser, Subcommand};
use wayrs_protocols::wlr_gamma_control_unstable_v1::ZwlrGammaControlManagerV1;

use color::Color;

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

struct State {
    outputs: Vec<wayland::Output>,
    gamma_manager: ZwlrGammaControlManagerV1,
    new_output_names: Vec<String>,
    output_names_to_delete: Vec<String>,
}

impl State {
    pub fn color(&self) -> Color {
        let color = self
            .outputs
            .iter()
            .fold(Color::default(), |color, output| Color {
                temp: color.temp + output.color().temp,
                gamma: color.gamma + output.color().gamma,
                brightness: color.brightness + output.color().brightness,
                inverted: color.inverted && output.color().inverted,
            });

        Color {
            temp: color.temp / self.outputs.len() as u16,
            gamma: color.gamma / self.outputs.len() as f64,
            brightness: color.brightness / self.outputs.len() as f64,
            inverted: color.inverted,
        }
    }

    pub fn color_changed(&self) -> bool {
        self.outputs.iter().any(|output| output.color_changed())
    }

    pub fn set_inverted(&mut self, inverted: bool) {
        for output in self.outputs.iter_mut() {
            output.set_color(Color {
                inverted,
                ..output.color()
            });
        }
    }

    pub fn set_brightness(&mut self, brightness: f64) {
        for output in self.outputs.iter_mut() {
            output.set_color(Color {
                brightness,
                ..output.color()
            });
        }
    }

    pub fn set_temperature(&mut self, temp: u16) {
        for output in self.outputs.iter_mut() {
            output.set_color(Color {
                temp,
                ..output.color()
            });
        }
    }

    pub fn set_gamma(&mut self, gamma: f64) {
        for output in self.outputs.iter_mut() {
            output.set_color(Color {
                gamma,
                ..output.color()
            });
        }
    }
}

fn main() -> anyhow::Result<()> {
    let commnad = Cli::parse().command.unwrap_or(Command::Run);
    let dbus_server = dbus_server::DbusServer::new()?;

    match commnad {
        Command::Run => {
            if let Some(mut dbus_server) = dbus_server {
                let (mut wayland, mut state) = wayland::Wayland::new()?;

                let mut fds = [pollin(&dbus_server), pollin(&wayland)];

                loop {
                    poll(&mut fds)?;
                    if fds[0].revents != 0 {
                        dbus_server.poll(&mut state)?;
                    }
                    if fds[1].revents != 0 || state.color_changed() {
                        wayland.poll(&mut state)?;
                    }
                }
            } else {
                eprintln!("wl-gammarelay-rs is already running");
            }
        }
        Command::Watch { format } => {
            let mut dbus_client = dbus_client::DbusClient::new(format, dbus_server.is_none())?;
            if let Some(mut dbus_server) = dbus_server {
                let (mut wayland, mut state) = wayland::Wayland::new()?;

                let mut fds = [pollin(&dbus_server), pollin(&wayland), pollin(&dbus_client)];

                loop {
                    poll(&mut fds)?;
                    if fds[0].revents != 0 {
                        dbus_server.poll(&mut state)?;
                    }
                    if fds[1].revents != 0 || state.color_changed() {
                        wayland.poll(&mut state)?;
                    }
                    if fds[2].revents != 0 {
                        dbus_client.run(false)?;
                    }
                }
            } else {
                dbus_client.run(true)?;
            }
        }
    }

    Ok(())
}

fn pollin(fd: &impl AsRawFd) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }
}

fn poll(fds: &mut [libc::pollfd]) -> io::Result<()> {
    loop {
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) } == -1 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        return Ok(());
    }
}
