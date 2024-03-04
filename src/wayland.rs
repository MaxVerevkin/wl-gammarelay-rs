use wayrs_client::protocol::*;
use wayrs_client::{EventCtx, IoMode};
use wayrs_protocols::wlr_gamma_control_unstable_v1::*;

use anyhow::Result;

use wayrs_client::cstr;
use wayrs_client::global::*;
use wayrs_client::proxy::Proxy;
use wayrs_client::Connection;

use std::io::ErrorKind;
use std::os::fd::{AsRawFd, RawFd};

use crate::color::{colorramp_fill, Color};
use crate::State;

pub struct Wayland {
    conn: Connection<State>,
}

impl AsRawFd for Wayland {
    fn as_raw_fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }
}

impl Wayland {
    pub fn new() -> Result<(Self, State)> {
        let (mut conn, globals) = Connection::connect_and_collect_globals()?;
        conn.add_registry_cb(wl_registry_cb);

        let gamma_manager = globals.bind(&mut conn, 1)?;

        let outputs = globals
            .iter()
            .filter(|g| g.is::<WlOutput>())
            .map(|output| Output::bind(&mut conn, output, gamma_manager))
            .collect();

        let state = State {
            color: Default::default(),
            color_changed: false,
            outputs,
            gamma_manager,
        };

        conn.flush(IoMode::Blocking)?;

        Ok((Self { conn }, state))
    }

    pub fn poll(&mut self, state: &mut State) -> Result<()> {
        match self.conn.recv_events(IoMode::NonBlocking) {
            Ok(()) => self.conn.dispatch_events(state),
            Err(e) if e.kind() == ErrorKind::WouldBlock => (),
            Err(e) => return Err(e.into()),
        }

        if state.color_changed {
            state.color_changed = false;
            state
                .outputs
                .iter_mut()
                .try_for_each(|o| o.set_color(&mut self.conn, state.color))?;
        }

        self.conn.flush(IoMode::Blocking)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Output {
    reg_name: u32,
    wl: WlOutput,
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
        let output = global.bind(conn, 1..=3).unwrap();
        Self {
            reg_name: global.name,
            wl: output,
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
        let file = shmemfdrs2::create_shmem(cstr!("/ramp-buffer"))?;
        file.set_len(self.ramp_size as u64 * 6)?;
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
