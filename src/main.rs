mod color;
mod dbus_client;
mod dbus_server;
mod wayland;

use std::io;
use std::os::fd::AsRawFd;

use clap::{Parser, Subcommand};
use dbus_server::DbusServer;
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

pub struct WaylandState {
    outputs: Vec<wayland::Output>,
    gamma_manager: ZwlrGammaControlManagerV1,
}

pub struct State {
    pub wayland_state: WaylandState,
    pub dbus_server: DbusServer,
}

impl WaylandState {
    pub fn output_by_reg_name(&self, reg_name: u32) -> Option<&wayland::Output> {
        self.outputs
            .iter()
            .find(|output| output.reg_name() == reg_name)
    }

    pub fn mut_output_by_reg_name(&mut self, reg_name: u32) -> Option<&mut wayland::Output> {
        self.outputs
            .iter_mut()
            .find(|output| output.reg_name() == reg_name)
    }

    /// Returns the average color of all outputs, or the default color if there are no outputs
    pub fn color(&self) -> Color {
        if self.outputs.is_empty() {
            Color::default()
        } else {
            let color = self.outputs.iter().fold(
                Color {
                    inverted: true,
                    brightness: 0.0,
                    temp: 0,
                    gamma: 0.0,
                },
                |color, output| {
                    let output_color = output.color();
                    Color {
                        inverted: color.inverted && output_color.inverted,
                        brightness: color.brightness + output_color.brightness,
                        temp: color.temp + output_color.temp,
                        gamma: color.gamma + output_color.gamma,
                    }
                },
            );

            Color {
                temp: color.temp / self.outputs.len() as u16,
                gamma: color.gamma / self.outputs.len() as f64,
                brightness: color.brightness / self.outputs.len() as f64,
                inverted: color.inverted,
            }
        }
    }

    pub fn color_changed(&self) -> bool {
        self.outputs.iter().any(|output| output.color_changed())
    }

    pub fn set_inverted(&mut self, inverted: bool) {
        for output in &mut self.outputs {
            let color = output.color();
            output.set_color(Color { inverted, ..color });
        }
    }

    pub fn set_brightness(&mut self, brightness: f64) {
        for output in &mut self.outputs {
            let color = output.color();
            output.set_color(Color {
                brightness,
                ..color
            });
        }
    }

    /// Returns `true` if any output was updated
    pub fn update_brightness(&mut self, delta: f64) -> bool {
        let mut updated = false;
        for output in &mut self.outputs {
            let color = output.color();
            let brightness = (color.brightness + delta).clamp(0.0, 1.0);
            if brightness != color.brightness {
                updated = true;
                output.set_color(Color {
                    brightness,
                    ..color
                });
            }
        }

        updated
    }

    pub fn set_temperature(&mut self, temp: u16) {
        for output in &mut self.outputs {
            let color = output.color();
            output.set_color(Color { temp, ..color });
        }
    }

    /// Returns `true` if any output was updated
    pub fn update_temperature(&mut self, delta: i16) -> bool {
        let mut updated = false;
        for output in &mut self.outputs {
            let color = output.color();
            let temp = (color.temp as i16 + delta).clamp(1_000, 10_000) as u16;
            if temp != color.temp {
                updated = true;
                output.set_color(Color { temp, ..color });
            }
        }

        updated
    }

    pub fn set_gamma(&mut self, gamma: f64) {
        for output in &mut self.outputs {
            let color = output.color();
            output.set_color(Color { gamma, ..color });
        }
    }

    /// Returns `true` if any output was updated
    pub fn update_gamma(&mut self, delta: f64) -> bool {
        let mut updated = false;
        for output in &mut self.outputs {
            let color = output.color();
            let gamma = (output.color().gamma + delta).max(0.1);
            if gamma != color.gamma {
                updated = true;
                output.set_color(Color { gamma, ..color });
            }
        }

        updated
    }
}

fn main() -> anyhow::Result<()> {
    let command = Cli::parse().command.unwrap_or(Command::Run);
    let dbus_server = dbus_server::DbusServer::new()?;

    match command {
        Command::Run => {
            if let Some(dbus_server) = dbus_server {
                let (mut wayland, wayland_state) = wayland::Wayland::new()?;

                let mut fds = [pollin(&dbus_server), pollin(&wayland)];
                let mut state = State {
                    wayland_state,
                    dbus_server,
                };

                loop {
                    poll(&mut fds)?;
                    if fds[0].revents != 0 {
                        state.dbus_server.poll(&mut state.wayland_state)?;
                    }
                    if fds[1].revents != 0 || state.wayland_state.color_changed() {
                        state = wayland.poll(state)?;
                    }
                }
            } else {
                eprintln!("wl-gammarelay-rs is already running");
            }
        }
        Command::Watch { format } => {
            let mut dbus_client = dbus_client::DbusClient::new(format, dbus_server.is_none())?;
            if let Some(dbus_server) = dbus_server {
                let (mut wayland, state) = wayland::Wayland::new()?;

                let mut fds = [pollin(&dbus_server), pollin(&wayland), pollin(&dbus_client)];
                let mut state = State {
                    wayland_state: state,
                    dbus_server,
                };

                loop {
                    poll(&mut fds)?;
                    if fds[0].revents != 0 {
                        state.dbus_server.poll(&mut state.wayland_state)?;
                    }
                    if fds[1].revents != 0 || state.wayland_state.color_changed() {
                        state = wayland.poll(state)?;
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
