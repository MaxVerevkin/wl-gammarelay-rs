use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1 as gamma_control_manager, zwlr_gamma_control_v1 as gamma_control,
};

use gamma_control::ZwlrGammaControlV1;
use gamma_control_manager::ZwlrGammaControlManagerV1;

use anyhow::Result;

use std::ffi::CStr;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

use crate::color::{colorramp_fill, Color};

#[derive(Debug)]
pub enum Request {
    SetColor(Color),
}

pub async fn run(mut rx: mpsc::Receiver<Request>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let mut async_fd = AsyncFd::new(event_queue.prepare_read()?.connection_fd().as_raw_fd())?;

    let gamma_manager = globals.bind(&qh, 1..=1, ())?;

    let outputs = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| Output::bind(global.name, &gamma_manager, globals.registry(), &qh))
            .collect()
    });

    let mut state = AppData {
        color: Default::default(),
        pending_updates: false,
        outputs,
        gamma_manager,
    };

    event_queue.flush()?;

    loop {
        tokio::select! {
            readable = async_fd.readable_mut() => {
                // FIXME: use readable.try_io()
                readable?.clear_ready();
                event_queue.prepare_read()?.read()?;
                event_queue.dispatch_pending(&mut state)?;
                event_queue.flush()?;
            }
            Some(request) = rx.recv() => {
                let Request::SetColor(color) = request;
                state.set_color(color);
            }
        }

        if state.pending_updates {
            state.apply_color()?;
            event_queue.flush()?;
        }
    }
}

#[derive(Debug)]
struct AppData {
    color: Color,
    pending_updates: bool,
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
        name: u32,
        gamma_manager: &gamma_control_manager::ZwlrGammaControlManagerV1,
        registry: &wl_registry::WlRegistry,
        qh: &QueueHandle<AppData>,
    ) -> Self {
        eprintln!("New output: {}", name);
        let output = registry.bind(name, 1, qh, ());
        Self {
            name,
            color: Default::default(),
            gamma_control: gamma_manager.get_gamma_control(&output, qh, ()),
            ramp_size: 0,
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        eprintln!("Output dropped: {}", self.name);
        self.gamma_control.destroy();
    }
}

/// Convert a slice of bytes to a slice of shorts (u16)
///
/// # Safety
///
/// - Same as [std::slice::from_raw_parts_mut]
unsafe fn bytes_to_shorts<'a>(bytes: &'a mut [u8]) -> &'a mut [u16] {
    let shorts_len = bytes.len() / 2;
    std::slice::from_raw_parts_mut::<'a, u16>(bytes.as_mut_ptr() as _, shorts_len)
}

impl AppData {
    fn set_color(&mut self, color: Color) {
        self.color = color;
        self.pending_updates = true;
    }

    fn apply_color(&mut self) -> anyhow::Result<()> {
        self.pending_updates = false;
        for output in &mut self.outputs {
            if output.ramp_size == 0 {
                self.pending_updates = true;
                continue;
            }
            if self.color != output.color {
                let fd = shmemfdrs::create_shmem(
                    CStr::from_bytes_with_nul(b"/ramp-buffer\0").unwrap(),
                    output.ramp_size * 6,
                );
                let file = unsafe { File::from_raw_fd(fd) };
                let mut mmap = unsafe { memmap::MmapMut::map_mut(&file)? };
                let buf = unsafe { bytes_to_shorts(&mut mmap) };
                let (r, rest) = buf.split_at_mut(output.ramp_size);
                let (g, b) = rest.split_at_mut(output.ramp_size);
                colorramp_fill(r, g, b, output.ramp_size, self.color);
                output.gamma_control.set_gamma(fd);
                output.color = self.color;
            }
        }
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<AppData>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version: _,
            } => {
                if interface == "wl_output" {
                    state
                        .outputs
                        .push(Output::bind(name, &state.gamma_manager, registry, qh));
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain_mut(|output| output.name != name);
            }
            _ => (),
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Ignore all events
    }
}

impl Dispatch<gamma_control_manager::ZwlrGammaControlManagerV1, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &gamma_control_manager::ZwlrGammaControlManagerV1,
        _: gamma_control_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // No events
    }
}

impl Dispatch<gamma_control::ZwlrGammaControlV1, ()> for AppData {
    fn event(
        state: &mut Self,
        gamma_ctrl: &gamma_control::ZwlrGammaControlV1,
        event: gamma_control::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            gamma_control::Event::GammaSize { size } => {
                let output = state
                    .outputs
                    .iter_mut()
                    .find(|o| &o.gamma_control == gamma_ctrl)
                    .expect("Received event for unknown output");
                eprintln!("Output {}: ramp_size = {size}", output.name);
                output.ramp_size = size as usize;
            }
            gamma_control::Event::Failed => {
                let output_index = state
                    .outputs
                    .iter()
                    .position(|o| &o.gamma_control == gamma_ctrl)
                    .expect("Received event for unknown output");
                let output = state.outputs.swap_remove(output_index);
                eprintln!("Output {}: gamma_control::Event::Failed", output.name);
            }
            _ => (),
        }
    }
}
