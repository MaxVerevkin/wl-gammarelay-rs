mod gamma_protocol {
    use wayrs_client;
    use wayrs_client::protocol::*;
    wayrs_scanner::generate!("wlr-gamma-control-unstable-v1.xml");
}

use gamma_protocol::*;
use wayrs_client::protocol::*;

use anyhow::Result;

use wayrs_client::connection::Connection;
use wayrs_client::cstr;
use wayrs_client::global::{Global, GlobalExt, GlobalsExt};
use wayrs_client::proxy::{Dispatch, Dispatcher};

use std::fs::File;
use std::os::unix::io::FromRawFd;

use tokio::sync::mpsc;

use crate::color::{colorramp_fill, Color};

#[derive(Debug)]
pub enum Request {
    SetColor(Color),
}

pub async fn run(mut rx: mpsc::Receiver<Request>) -> Result<()> {
    let mut conn = Connection::connect()?;
    let globals = conn.async_collect_initial_globals().await?;

    let gamma_manager = globals.bind(&mut conn, 1..=1)?;

    let outputs = globals
        .iter()
        .filter(|g| g.is::<WlOutput>())
        .map(|output| Output::bind(&mut conn, output, gamma_manager))
        .collect();

    let mut state = AppData {
        color: Default::default(),
        outputs,
        gamma_manager,
    };

    conn.async_flush().await?;

    loop {
        tokio::select! {
            recv_events = conn.async_recv_events() => {
                recv_events?;
                conn.dispatch_events(&mut state)?;
                conn.async_flush().await?;
            }
            Some(request) = rx.recv() => {
                let Request::SetColor(color) = request;
                state.color = color;
                state.outputs.iter_mut().try_for_each(|o| o.set_color(&mut conn, color))?;
                conn.async_flush().await?;
            }
        }
    }
}

#[derive(Debug)]
struct AppData {
    color: Color,
    outputs: Vec<Output>,
    gamma_manager: ZwlrGammaControlManagerV1,
}

#[derive(Debug)]
struct Output {
    name: u32,
    color: Color,
    gamma_control: ZwlrGammaControlV1,
    ramp_size: usize,
}

impl Output {
    fn bind(
        conn: &mut Connection<AppData>,
        global: &Global,
        gamma_manager: ZwlrGammaControlManagerV1,
    ) -> Self {
        eprintln!("New output: {}", global.name);
        let output = global.bind(conn, 1..=1).unwrap();
        Self {
            name: global.name,
            color: Default::default(),
            gamma_control: gamma_manager.get_gamma_control(conn, output),
            ramp_size: 0,
        }
    }

    fn set_color(&mut self, conn: &mut Connection<AppData>, color: Color) -> Result<()> {
        if self.ramp_size == 0 || color == self.color {
            return Ok(());
        }

        self.color = color;
        let fd = shmemfdrs::create_shmem(cstr!("/ramp-buffer"), self.ramp_size * 6);
        let file = unsafe { File::from_raw_fd(fd) };
        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };
        let buf = bytes_to_shorts(&mut mmap);
        let (r, rest) = buf.split_at_mut(self.ramp_size);
        let (g, b) = rest.split_at_mut(self.ramp_size);
        colorramp_fill(r, g, b, self.ramp_size, self.color);
        self.gamma_control.set_gamma(conn, file.into());
        Ok(())
    }
}

/// Convert a slice of bytes to a slice of shorts (u16)
fn bytes_to_shorts<'a>(bytes: &'a mut [u8]) -> &'a mut [u16] {
    let shorts_len = bytes.len() / 2;
    unsafe { std::slice::from_raw_parts_mut::<'a, u16>(bytes.as_mut_ptr() as _, shorts_len) }
}

impl Dispatcher for AppData {
    type Error = anyhow::Error;
}

impl Dispatch<WlRegistry> for AppData {
    fn try_event(
        &mut self,
        conn: &mut Connection<Self>,
        _: WlRegistry,
        event: wl_registry::Event,
    ) -> Result<()> {
        match event {
            wl_registry::Event::Global(global) if global.is::<WlOutput>() => {
                let mut output = Output::bind(conn, &global, self.gamma_manager);
                output.set_color(conn, self.color)?;
                self.outputs.push(output);
            }
            wl_registry::Event::GlobalRemove(name) => {
                if let Some(output_index) = self.outputs.iter().position(|o| o.name == name) {
                    let output = self.outputs.swap_remove(output_index);
                    output.gamma_control.destroy(conn);
                    eprintln!("Output {} removed", output.name);
                }
            }
            _ => (),
        }
        Ok(())
    }
}

impl Dispatch<ZwlrGammaControlV1> for AppData {
    fn try_event(
        &mut self,
        conn: &mut Connection<Self>,
        gamma_ctrl: ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
    ) -> Result<()> {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize(size) => {
                let output = self
                    .outputs
                    .iter_mut()
                    .find(|o| o.gamma_control == gamma_ctrl)
                    .expect("Received event for unknown output");
                output.ramp_size = size as usize;
                output.set_color(conn, self.color)?;
                eprintln!("Output {}: ramp_size = {}", output.name, size);
            }
            zwlr_gamma_control_v1::Event::Failed => {
                let output_index = self
                    .outputs
                    .iter()
                    .position(|o| o.gamma_control == gamma_ctrl)
                    .expect("Received event for unknown output");
                let output = self.outputs.swap_remove(output_index);
                output.gamma_control.destroy(conn);
                eprintln!("Output {}: gamma_control::Event::Failed", output.name);
            }
        }
        Ok(())
    }
}

// Don't care
impl Dispatch<WlOutput> for AppData {}

// No events
impl Dispatch<ZwlrGammaControlManagerV1> for AppData {}
