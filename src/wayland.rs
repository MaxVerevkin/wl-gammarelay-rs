use wayland_client::{
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1 as gamma_control_manager, zwlr_gamma_control_v1 as gamma_control,
};

use anyhow::Result;

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

use crate::color::{colorramp_fill, Color};

#[derive(Debug)]
pub enum Request {
    SetColor(Color),
}

pub async fn run(mut rx: mpsc::Receiver<Request>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let mut async_fd = AsyncFd::new(event_queue.prepare_read()?.connection_fd())?;
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ())?;

    let mut data = AppData {
        color: Default::default(),
        pending_updates: false,
        outputs: HashMap::new(),
        manager: None,
    };

    event_queue.dispatch_pending(&mut data)?;
    event_queue.flush()?;

    loop {
        tokio::select! {
            readable = async_fd.readable_mut() => {
                // FIXME: use readable.try_io()
                readable?.clear_ready();
                event_queue.prepare_read()?.read()?;
                event_queue.dispatch_pending(&mut data)?;
                event_queue.flush()?;
            }
            Some(request) = rx.recv() => {
                let Request::SetColor(color) = request;
                data.set_color(color);
            }
        }

        if data.pending_updates {
            data.apply_color()?;
            event_queue.flush()?;
        }
    }
}

#[derive(Debug)]
struct AppData {
    color: Color,
    pending_updates: bool,
    outputs: HashMap<u32, Output>,
    manager: Option<gamma_control_manager::ZwlrGammaControlManagerV1>,
}

#[derive(Debug)]
struct Output {
    color: Color,
    gamma_control: Option<(gamma_control::ZwlrGammaControlV1, Arc<AtomicU32>)>,
    output: wl_output::WlOutput,
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
        for output in self.outputs.values_mut() {
            if let Some((gamma_control, ramp_size)) = &output.gamma_control {
                let ramp_size = ramp_size.load(Ordering::SeqCst) as usize;
                if ramp_size == 0 {
                    self.pending_updates = true;
                    continue;
                }
                if self.color != output.color {
                    let fd = shmemfdrs::create_shmem(
                        CStr::from_bytes_with_nul(b"/ramp-buffer\0").unwrap(),
                        ramp_size * 6,
                    );
                    let file = unsafe { File::from_raw_fd(fd) };
                    let mut mmap = unsafe { memmap::MmapMut::map_mut(&file)? };
                    let buf = unsafe { bytes_to_shorts(&mut mmap) };
                    let (r, rest) = buf.split_at_mut(ramp_size);
                    let (g, b) = rest.split_at_mut(ramp_size);
                    colorramp_fill(r, g, b, ramp_size, self.color);
                    gamma_control.set_gamma(fd);
                    output.color = self.color;
                }
            }
        }
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppData>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    eprintln!("New output: {name}");
                    state.outputs.insert(
                        name,
                        Output {
                            color: Default::default(),
                            gamma_control: None,
                            output: registry.bind(name, version, qh, name).unwrap(),
                        },
                    );
                }
                "zwlr_gamma_control_manager_v1" => {
                    eprintln!("Found gamma control manager");
                    state.manager = Some(registry.bind(name, version, qh, ()).unwrap());
                }
                _ => (),
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(output) = state.outputs.remove(&name) {
                    eprintln!("Output removed: {name}");
                    output.output.release();
                    if let Some((gamma_control, _)) = output.gamma_control {
                        gamma_control.destroy();
                    }
                } else {
                    eprintln!("Unknown name removed: {name}");
                }
            }
            _ => (),
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for AppData {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        e: wl_output::Event,
        output_name: &u32,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Done = e {
            if let Some(manager) = &state.manager {
                let ramp_size = Arc::new(AtomicU32::new(0));
                let gamma = manager
                    .get_gamma_control(output, qh, (*output_name, ramp_size.clone()))
                    .unwrap();
                state.outputs.get_mut(output_name).unwrap().gamma_control =
                    Some((gamma, ramp_size));
                state.pending_updates = true;
                eprintln!("Output {output_name} initialized");
            } else {
                eprintln!("Cannot initialize output {output_name}: no gamma control manager");
            }
        }
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

impl Dispatch<gamma_control::ZwlrGammaControlV1, (u32, Arc<AtomicU32>)> for AppData {
    fn event(
        state: &mut Self,
        gamma_ctrl: &gamma_control::ZwlrGammaControlV1,
        event: gamma_control::Event,
        (name, ramp_size): &(u32, Arc<AtomicU32>),
        _conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let gamma_control::Event::GammaSize { size } = event {
            eprintln!("Output {name}: ramp_size = {size}");
            ramp_size.store(size, Ordering::SeqCst)
        } else if let gamma_control::Event::Failed = event {
            eprintln!("Output {name} failed: destroying it's gamma control");
            state.outputs.get_mut(name).unwrap().gamma_control = None;
            gamma_ctrl.destroy();
        }
    }
}
