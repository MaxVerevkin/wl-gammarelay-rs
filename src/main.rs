use wayland_client::{
    protocol::{wl_output, wl_registry},
    Connection, ConnectionHandle, Dispatch, QueueHandle,
};
use wayland_protocols::wlr::unstable::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1 as gamma_control_manager, zwlr_gamma_control_v1 as gamma_control,
};

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

mod dbus;
use dbus::Request;
use dbus::Server;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, mut rx) = mpsc::channel(16);
    Server::new(tx).run().await?;

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    temp: u16,
    brightness: f64,
}

impl Default for Color {
    fn default() -> Self {
        Self {
            temp: 6500,
            brightness: 1.0,
        }
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

/// [Black body radiation color](https://en.wikipedia.org/wiki/Black-body_radiation) mapped by
/// temperature in the range [1_000,10_100].
///
/// Refer to <https://gitlab.com/chinstrap/gammastep/-/blob/master/README-colorramp> for more info.
const BLACKBODY_COLOR: &[f64] = &[
    1.00000000, 0.18172716, 0.00000000, 1.00000000, 0.25503671, 0.00000000, 1.00000000, 0.30942099,
    0.00000000, 1.00000000, 0.35357379, 0.00000000, 1.00000000, 0.39091524, 0.00000000, 1.00000000,
    0.42322816, 0.00000000, 1.00000000, 0.45159884, 0.00000000, 1.00000000, 0.47675916, 0.00000000,
    1.00000000, 0.49923747, 0.00000000, 1.00000000, 0.51943421, 0.00000000, 1.00000000, 0.54360078,
    0.08679949, 1.00000000, 0.56618736, 0.14065513, 1.00000000, 0.58734976, 0.18362641, 1.00000000,
    0.60724493, 0.22137978, 1.00000000, 0.62600248, 0.25591950, 1.00000000, 0.64373109, 0.28819679,
    1.00000000, 0.66052319, 0.31873863, 1.00000000, 0.67645822, 0.34786758, 1.00000000, 0.69160518,
    0.37579588, 1.00000000, 0.70602449, 0.40267128, 1.00000000, 0.71976951, 0.42860152, 1.00000000,
    0.73288760, 0.45366838, 1.00000000, 0.74542112, 0.47793608, 1.00000000, 0.75740814, 0.50145662,
    1.00000000, 0.76888303, 0.52427322, 1.00000000, 0.77987699, 0.54642268, 1.00000000, 0.79041843,
    0.56793692, 1.00000000, 0.80053332, 0.58884417, 1.00000000, 0.81024551, 0.60916971, 1.00000000,
    0.81957693, 0.62893653, 1.00000000, 0.82854786, 0.64816570, 1.00000000, 0.83717703, 0.66687674,
    1.00000000, 0.84548188, 0.68508786, 1.00000000, 0.85347859, 0.70281616, 1.00000000, 0.86118227,
    0.72007777, 1.00000000, 0.86860704, 0.73688797, 1.00000000, 0.87576611, 0.75326132, 1.00000000,
    0.88267187, 0.76921169, 1.00000000, 0.88933596, 0.78475236, 1.00000000, 0.89576933, 0.79989606,
    1.00000000, 0.90198230, 0.81465502, 1.00000000, 0.90963069, 0.82838210, 1.00000000, 0.91710889,
    0.84190889, 1.00000000, 0.92441842, 0.85523742, 1.00000000, 0.93156127, 0.86836903, 1.00000000,
    0.93853986, 0.88130458, 1.00000000, 0.94535695, 0.89404470, 1.00000000, 0.95201559, 0.90658983,
    1.00000000, 0.95851906, 0.91894041, 1.00000000, 0.96487079, 0.93109690, 1.00000000, 0.97107439,
    0.94305985, 1.00000000, 0.97713351, 0.95482993, 1.00000000, 0.98305189, 0.96640795, 1.00000000,
    0.98883326, 0.97779486, 1.00000000, 0.99448139, 0.98899179, 1.00000000, 1.00000000, 1.00000000,
    0.98947904, 0.99348723, 1.00000000, 0.97940448, 0.98722715, 1.00000000, 0.96975025, 0.98120637,
    1.00000000, 0.96049223, 0.97541240, 1.00000000, 0.95160805, 0.96983355, 1.00000000, 0.94303638,
    0.96443333, 1.00000000, 0.93480451, 0.95923080, 1.00000000, 0.92689056, 0.95421394, 1.00000000,
    0.91927697, 0.94937330, 1.00000000, 0.91194747, 0.94470005, 1.00000000, 0.90488690, 0.94018594,
    1.00000000, 0.89808115, 0.93582323, 1.00000000, 0.89151710, 0.93160469, 1.00000000, 0.88518247,
    0.92752354, 1.00000000, 0.87906581, 0.92357340, 1.00000000, 0.87315640, 0.91974827, 1.00000000,
    0.86744421, 0.91604254, 1.00000000, 0.86191983, 0.91245088, 1.00000000, 0.85657444, 0.90896831,
    1.00000000, 0.85139976, 0.90559011, 1.00000000, 0.84638799, 0.90231183, 1.00000000, 0.84153180,
    0.89912926, 1.00000000, 0.83682430, 0.89603843, 1.00000000, 0.83225897, 0.89303558, 1.00000000,
    0.82782969, 0.89011714, 1.00000000, 0.82353066, 0.88727974, 1.00000000, 0.81935641, 0.88452017,
    1.00000000, 0.81530175, 0.88183541, 1.00000000, 0.81136180, 0.87922257, 1.00000000, 0.80753191,
    0.87667891, 1.00000000, 0.80380769, 0.87420182, 1.00000000, 0.80018497, 0.87178882, 1.00000000,
    0.79665980, 0.86943756, 1.00000000, 0.79322843, 0.86714579, 1.00000000, 0.78988728, 0.86491137,
    1.00000000, 0.78663296, 0.86273225, 1.00000000,
];

fn interpolate_color(a: f64, c1: &[f64], c2: &[f64]) -> [f64; 3] {
    [
        (1.0 - a) * c1[0] + a * c2[0],
        (1.0 - a) * c1[1] + a * c2[1],
        (1.0 - a) * c1[2] + a * c2[2],
    ]
}

fn colorramp_fill(r: &mut [u16], g: &mut [u16], b: &mut [u16], ramp_size: usize, color: Color) {
    let color_i = ((color.temp as usize - 1000) / 100) * 3;
    let [white_r, white_g, white_b] = interpolate_color(
        (color.temp % 100) as f64 / 100.0,
        &BLACKBODY_COLOR[color_i..],
        &BLACKBODY_COLOR[(color_i + 3)..],
    );

    let step = u16::MAX as f64 * color.brightness / (ramp_size - 1) as f64;
    for i in 0..ramp_size {
        let v = step * i as f64;
        r[i] = (v * white_r) as u16;
        g[i] = (v * white_g) as u16;
        b[i] = (v * white_b) as u16;
    }
}
