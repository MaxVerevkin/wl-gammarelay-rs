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
use crate::{State, WaylandState};

pub struct Wayland {
    conn: Connection<State>,
}

impl AsRawFd for Wayland {
    fn as_raw_fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }
}

impl Wayland {
    pub fn new() -> Result<(Self, WaylandState)> {
        let (mut conn, globals) = Connection::connect_and_collect_globals()?;
        conn.add_registry_cb(wl_registry_cb);

        let gamma_manager = globals.bind(&mut conn, 1)?;

        let outputs = globals
            .iter()
            .filter(|g| g.is::<WlOutput>())
            .map(|output| Output::bind(&mut conn, output, gamma_manager))
            .collect();

        let state = WaylandState {
            outputs,
            gamma_manager,
        };

        conn.flush(IoMode::Blocking)?;

        Ok((Self { conn }, state))
    }

    pub fn poll(&mut self, mut state: State) -> Result<State> {
        match self.conn.recv_events(IoMode::NonBlocking) {
            Ok(()) => self.conn.dispatch_events(&mut state),
            Err(e) if e.kind() == ErrorKind::WouldBlock => (),
            Err(e) => return Err(e.into()),
        }

        for output in &mut state.wayland_state.outputs {
            if output.color_changed {
                output.update_displayed_color(&mut self.conn)?;
            }
        }
        self.conn.flush(IoMode::Blocking)?;
        Ok(state)
    }
}

#[derive(Debug)]
pub struct Output {
    reg_name: u32,
    wl: WlOutput,
    name: Option<String>,
    color: Color,
    gamma_control: ZwlrGammaControlV1,
    ramp_size: usize,
    color_changed: bool,
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
            color_changed: true,
        }
    }

    fn destroy(self, conn: &mut Connection<State>) {
        eprintln!("Output {} removed", self.reg_name);
        self.gamma_control.destroy(conn);
        if self.wl.version() >= 3 {
            self.wl.release(conn);
        }
    }

    pub fn reg_name(&self) -> u32 {
        self.reg_name
    }

    pub fn name(&self) -> &str {
        self.name.as_ref().unwrap()
    }

    pub fn color(&self) -> Color {
        self.color
    }

    pub fn color_changed(&self) -> bool {
        self.color_changed
    }

    pub fn set_color(&mut self, color: Color) {
        if self.ramp_size == 0 || color == self.color {
            return;
        }

        self.color = color;
        self.color_changed = true;
    }

    pub fn object_path(&self) -> String {
        format!("/outputs/{}", self.name().replace('-', "_"))
    }

    fn update_displayed_color(&mut self, conn: &mut Connection<State>) -> Result<()> {
        if self.ramp_size == 0 {
            return Ok(());
        }

        let file = shmemfdrs2::create_shmem(cstr!("/ramp-buffer"))?;
        file.set_len(self.ramp_size as u64 * 6)?;
        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };
        let buf = bytemuck::cast_slice_mut::<u8, u16>(&mut mmap);
        let (r, rest) = buf.split_at_mut(self.ramp_size);
        let (g, b) = rest.split_at_mut(self.ramp_size);
        colorramp_fill(r, g, b, self.ramp_size, self.color);
        self.gamma_control.set_gamma(conn, file.into());

        self.color_changed = false;
        Ok(())
    }
}

fn wl_registry_cb(conn: &mut Connection<State>, state: &mut State, event: &wl_registry::Event) {
    match event {
        wl_registry::Event::Global(global) if global.is::<WlOutput>() => {
            let mut output = Output::bind(conn, global, state.wayland_state.gamma_manager);
            output.set_color(state.wayland_state.color());
            output.update_displayed_color(conn).unwrap();
            state.wayland_state.outputs.push(output);
        }
        wl_registry::Event::GlobalRemove(name) => {
            if let Some(output_index) = state
                .wayland_state
                .outputs
                .iter()
                .position(|o| o.reg_name == *name)
            {
                if let Some(output_name) = &state.wayland_state.outputs[output_index].name {
                    state.dbus_server.remove_output(output_name);
                }
                let output = state.wayland_state.outputs.swap_remove(output_index);
                output.destroy(conn);
            }
        }
        _ => (),
    }
}

fn gamma_control_cb(ctx: EventCtx<State, ZwlrGammaControlV1>) {
    let output_index = ctx
        .state
        .wayland_state
        .outputs
        .iter()
        .position(|o| o.gamma_control == ctx.proxy)
        .expect("Received event for unknown output");
    match ctx.event {
        zwlr_gamma_control_v1::Event::GammaSize(size) => {
            let output = &mut ctx.state.wayland_state.outputs[output_index];
            eprintln!("Output {}: ramp_size = {}", output.reg_name, size);
            output.ramp_size = size as usize;
            output.update_displayed_color(ctx.conn).unwrap();
        }
        zwlr_gamma_control_v1::Event::Failed => {
            let output = ctx.state.wayland_state.outputs.swap_remove(output_index);
            eprintln!("Output {}: gamma_control::Event::Failed", output.reg_name);
            if let Some(output_name) = &output.name {
                ctx.state.dbus_server.remove_output(output_name);
            }
            output.destroy(ctx.conn);
        }
        _ => (),
    }
}

fn wl_output_cb(ctx: EventCtx<State, WlOutput>) {
    if let wl_output::Event::Name(name) = ctx.event {
        let output = ctx
            .state
            .wayland_state
            .outputs
            .iter_mut()
            .find(|o| o.wl == ctx.proxy)
            .unwrap();
        let name = String::from_utf8(name.into_bytes()).expect("invalid output name");
        eprintln!("Output {}: name = {name:?}", output.reg_name);
        ctx.state.dbus_server.add_output(output.reg_name, &name);
        output.name = Some(name);
    }
}
