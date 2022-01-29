use wayland_client::{
    protocol::{wl_output, wl_registry},
    Connection, ConnectionHandle, Dispatch, QueueHandle,
};
use wayland_protocols::wlr::unstable::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1 as gamma_control_manager, zwlr_gamma_control_v1 as gamma_control,
};

use anyhow::Result;

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
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
    let display = conn.handle().display();
    let mut event_queue = conn.new_event_queue();
    let mut async_fd = AsyncFd::new(event_queue.prepare_read()?.connection_fd())?;
    let qh = event_queue.handle();
    let _registry = display.get_registry(&mut conn.handle(), &qh, ())?;

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
            data.apply_color(&mut conn.handle())?;
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

    fn apply_color(&mut self, conn: &mut ConnectionHandle) -> anyhow::Result<()> {
        self.pending_updates = false;
        for output in self.outputs.values_mut() {
            if let Some((gamma_control, ramp_size)) = &output.gamma_control {
                let ramp_size = ramp_size.load(Ordering::SeqCst) as usize;
                if ramp_size == 0 {
                    self.pending_updates = true;
                    continue;
                }
                if self.color != output.color {
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .open("/tmp/wl-gammarelay-rs-temp")?;
                    file.set_len(ramp_size as u64 * 6)?;
                    let mut mmap = unsafe { memmap::MmapMut::map_mut(&file)? };
                    let buf = unsafe { bytes_to_shorts(&mut *mmap) };
                    let (r, rest) = buf.split_at_mut(ramp_size);
                    let (g, b) = rest.split_at_mut(ramp_size);
                    colorramp_fill(r, g, b, ramp_size, self.color);
                    mmap.flush()?;
                    gamma_control.set_gamma(conn, file.as_raw_fd());
                    output.color = self.color;
                }
            }
        }
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry> for AppData {
    type UserData = ();

    fn event(
        &mut self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &Self::UserData,
        conn: &mut ConnectionHandle,
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
                    self.outputs.insert(
                        name,
                        Output {
                            color: Default::default(),
                            gamma_control: None,
                            output: registry.bind(conn, name, version, qh, name).unwrap(),
                        },
                    );
                }
                "zwlr_gamma_control_manager_v1" => {
                    eprintln!("Found gamma control manager");
                    self.manager = Some(registry.bind(conn, name, version, qh, ()).unwrap());
                }
                _ => (),
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(output) = self.outputs.remove(&name) {
                    eprintln!("Output removed: {name}");
                    output.output.release(conn);
                    if let Some((gamma_control, _)) = output.gamma_control {
                        gamma_control.destroy(conn);
                    }
                } else {
                    eprintln!("Unknown name removed: {name}");
                }
            }
            _ => (),
        }
    }
}

impl Dispatch<wl_output::WlOutput> for AppData {
    /// output_name
    type UserData = u32;

    fn event(
        &mut self,
        output: &wl_output::WlOutput,
        e: wl_output::Event,
        name: &Self::UserData,
        conn: &mut ConnectionHandle,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Done = e {
            if let Some(manager) = &self.manager {
                let ramp_size = Arc::new(AtomicU32::new(0));
                let gamma = manager
                    .get_gamma_control(conn, output, qh, (*name, ramp_size.clone()))
                    .unwrap();
                self.outputs.get_mut(name).unwrap().gamma_control = Some((gamma, ramp_size));
                self.pending_updates = true;
                eprintln!("Output {name} initialized");
            } else {
                eprintln!("Cannot initialize output {name}: no gamma control manager");
            }
        }
    }
}

impl Dispatch<gamma_control_manager::ZwlrGammaControlManagerV1> for AppData {
    type UserData = ();

    fn event(
        &mut self,
        _: &gamma_control_manager::ZwlrGammaControlManagerV1,
        _: gamma_control_manager::Event,
        _: &Self::UserData,
        _: &mut ConnectionHandle,
        _: &QueueHandle<Self>,
    ) {
        // No events
    }
}

impl Dispatch<gamma_control::ZwlrGammaControlV1> for AppData {
    /// (output_name, ramp_size)
    type UserData = (u32, Arc<AtomicU32>);

    fn event(
        &mut self,
        gamma_ctrl: &gamma_control::ZwlrGammaControlV1,
        event: gamma_control::Event,
        (name, ramp_size): &Self::UserData,
        conn: &mut ConnectionHandle,
        _: &QueueHandle<Self>,
    ) {
        if let gamma_control::Event::GammaSize { size } = event {
            eprintln!("Output {name}: ramp_size = {size}");
            ramp_size.store(size, Ordering::SeqCst)
        } else if let gamma_control::Event::Failed = event {
            eprintln!("Output {name} failed: destroying it's gamma control");
            self.outputs.get_mut(name).unwrap().gamma_control = None;
            gamma_ctrl.destroy(conn);
        }
    }
}
