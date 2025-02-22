mod color;
mod dbus_client;
mod dbus_server;
mod wayland;

use std::io;
use std::os::fd::{AsRawFd, RawFd};

use clap::{Parser, Subcommand};
use wayland::WaylandEvent;

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

fn main() -> anyhow::Result<()> {
    let command = Cli::parse().command.unwrap_or(Command::Run);
    match dbus_server::DbusServer::new()? {
        Some(mut dbus_server) => {
            let mut wayland = wayland::Wayland::new()?;
            let mut dbus_client = match command {
                Command::Run => None,
                Command::Watch { format } => Some(dbus_client::DbusClient::new(format, false)?),
            };
            let mut fds = [
                pollin(dbus_server.as_raw_fd()),
                pollin(wayland.as_raw_fd()),
                pollin(dbus_client.as_ref().map_or(-1, |x| x.as_raw_fd())),
            ];
            let fds_cnt = if dbus_client.is_some() { 3 } else { 2 };
            loop {
                while let Some(event) = wayland.next_event() {
                    match event {
                        WaylandEvent::NewOutput { reg_name, name } => {
                            dbus_server.add_output(reg_name, &name);
                        }
                        WaylandEvent::RemoveOutput { name } => {
                            dbus_server.remove_output(&name);
                        }
                    }
                }

                poll(&mut fds[..fds_cnt])?;
                if fds[0].revents != 0 {
                    dbus_server.poll(&mut wayland.state)?;
                }
                if fds[1].revents != 0 || wayland.state.color_changed() {
                    wayland.poll()?;
                }
                if fds[2].revents != 0 {
                    dbus_client.as_mut().unwrap().run(false)?;
                }
            }
        }
        None => match command {
            Command::Run => eprintln!("wl-gammarelay-rs is already running"),
            Command::Watch { format } => {
                let mut dbus_client = dbus_client::DbusClient::new(format, true)?;
                dbus_client.run(true)?;
            }
        },
    }

    Ok(())
}

impl wayland::WaylandState {
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
            if let Some(new_color) = output.color().with_updated_temp(delta) {
                updated = true;
                output.set_color(new_color);
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

fn pollin(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
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
