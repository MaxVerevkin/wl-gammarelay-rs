use wayrs_client::protocol::*;
use wayrs_client::EventCtx;
use wayrs_protocols::wlr_gamma_control_unstable_v1::*;

use anyhow::Result;

use wayrs_client::cstr;
use wayrs_client::global::*;
use wayrs_client::proxy::Proxy;
use wayrs_client::Connection;

use std::fs::File;
use std::os::unix::io::FromRawFd;

use tokio::sync::mpsc;

use crate::color::{colorramp_fill, Color};
use crate::dbus_server::RootServer;

#[derive(Debug)]
pub enum Request {
    SetColor { color: Color, output_name: String },
}

pub async fn run(
    mut rx: mpsc::Receiver<Request>,
    tx: mpsc::Sender<Request>,
    mut instance: zbus::Connection,
    root_server: RootServer,
) -> Result<()> {
    let (mut conn, globals) = Connection::async_connect_and_collect_globals().await?;
    conn.add_registry_cb(wl_registry_cb);

    let gamma_manager = globals.bind(&mut conn, 1)?;

    let outputs = globals
        .iter()
        .filter(|g| g.is::<WlOutput>())
        .map(|output| Output::bind(&mut conn, output, gamma_manager))
        .collect();

    let mut state = State {
        color: Default::default(),
        outputs,
        gamma_manager,
        new_output_names: Vec::new(),
        output_names_to_delete: Vec::new(),
    };

    loop {
        conn.async_flush().await?;

        tokio::select! {
            recv_events = conn.async_recv_events() => {
                recv_events?;
                conn.dispatch_events(&mut state);
                while let Some(output_name) = state.new_output_names.pop() {
                    root_server.add_output(&mut instance, tx.clone(), output_name).await?;
                }
                while let Some(output_name) = state.output_names_to_delete.pop() {
                    root_server.remove_output(&mut instance, output_name).await?;
                }
            }
            Some(request) = rx.recv() => {
                let Request::SetColor { color, output_name } = request;
                state.color = color;
                state
                    .outputs
                    .iter_mut()
                    .filter(|o| o.name.as_ref() == Some(&output_name))
                    .try_for_each(|o| o.set_color(&mut conn, color))?;
            }
        }
    }
}

#[derive(Debug)]
struct State {
    color: Color,
    outputs: Vec<Output>,
    gamma_manager: ZwlrGammaControlManagerV1,
    new_output_names: Vec<String>,
    output_names_to_delete: Vec<String>,
}

#[derive(Debug)]
struct Output {
    reg_name: u32,
    wl: WlOutput,
    name: Option<String>,
    color: Color,
    gamma_control: ZwlrGammaControlV1,
    ramp_size: usize,
}

impl Output {
    fn bind(
        conn: &mut Connection<State>,
        global: &Global,
        gamma_manager: ZwlrGammaControlManagerV1,
    ) -> Self {
        eprintln!("New output: {}", global.name);
        let output = global.bind_with_cb(conn, 4, wl_output_cb).unwrap();
        Self {
            reg_name: global.name,
            wl: output,
            name: None,
            color: Default::default(),
            gamma_control: gamma_manager.get_gamma_control_with_cb(conn, output, gamma_control_cb),
            ramp_size: 0,
        }
    }

    fn destroy(self, conn: &mut Connection<State>) {
        eprintln!("Output {} removed", self.reg_name);
        self.gamma_control.destroy(conn);
        if self.wl.version() >= 3 {
            self.wl.release(conn);
        }
    }

    fn set_color(&mut self, conn: &mut Connection<State>, color: Color) -> Result<()> {
        if self.ramp_size == 0 || color == self.color {
            return Ok(());
        }

        self.color = color;
        let fd = shmemfdrs::create_shmem(cstr!("/ramp-buffer"), self.ramp_size * 6);
        let file = unsafe { File::from_raw_fd(fd) };
        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };
        let buf = bytemuck::cast_slice_mut::<u8, u16>(&mut mmap);
        let (r, rest) = buf.split_at_mut(self.ramp_size);
        let (g, b) = rest.split_at_mut(self.ramp_size);
        colorramp_fill(r, g, b, self.ramp_size, self.color);
        self.gamma_control.set_gamma(conn, file.into());
        Ok(())
    }
}

fn wl_registry_cb(conn: &mut Connection<State>, state: &mut State, event: &wl_registry::Event) {
    match event {
        wl_registry::Event::Global(global) if global.is::<WlOutput>() => {
            let mut output = Output::bind(conn, global, state.gamma_manager);
            output.set_color(conn, state.color).unwrap();
            state.outputs.push(output);
        }
        wl_registry::Event::GlobalRemove(name) => {
            if let Some(output_index) = state.outputs.iter().position(|o| o.reg_name == *name) {
                let output = state.outputs.swap_remove(output_index);
                state
                    .output_names_to_delete
                    .push(output.name.clone().unwrap());
                output.destroy(conn);
            }
        }
        _ => (),
    }
}

fn gamma_control_cb(ctx: EventCtx<State, ZwlrGammaControlV1>) {
    let output_index = ctx
        .state
        .outputs
        .iter()
        .position(|o| o.gamma_control == ctx.proxy)
        .expect("Received event for unknown output");
    match ctx.event {
        zwlr_gamma_control_v1::Event::GammaSize(size) => {
            let output = &mut ctx.state.outputs[output_index];
            eprintln!("Output {}: ramp_size = {}", output.reg_name, size);
            output.ramp_size = size as usize;
            output.set_color(ctx.conn, ctx.state.color).unwrap();
        }
        zwlr_gamma_control_v1::Event::Failed => {
            let output = ctx.state.outputs.swap_remove(output_index);
            eprintln!("Output {}: gamma_control::Event::Failed", output.reg_name);
            output.destroy(ctx.conn);
        }
        _ => (),
    }
}

fn wl_output_cb(ctx: EventCtx<State, WlOutput>) {
    if let wl_output::Event::Name(name) = ctx.event {
        let output = ctx
            .state
            .outputs
            .iter_mut()
            .find(|o| o.wl == ctx.proxy)
            .unwrap();
        let name = String::from_utf8(name.into_bytes()).expect("invalid output name");
        eprintln!("Output {}: name = {name:?}", output.reg_name);
        ctx.state.new_output_names.push(name.clone());
        output.name = Some(name);
    }
}
