use std::{
    collections::HashMap,
    os::fd::{AsRawFd, RawFd},
};

use anyhow::Result;
use rustbus_service::rustbus::{
    self, connection::Timeout, get_session_bus_path, standard_messages,
    wire::unmarshal::traits::Variant as UnVariant, DuplexConn, MessageBuilder, MessageType,
};

pub struct DbusClient {
    format: String,
    conn: DuplexConn,
    temperature: u16,
    gamma: f64,
    brightness: f64,
    prev_output: Option<String>,
}

impl AsRawFd for DbusClient {
    fn as_raw_fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }
}

impl DbusClient {
    pub fn new(format: String, server_running: bool) -> Result<Self> {
        let mut conn = DuplexConn::connect_to_bus(get_session_bus_path()?, true)?;
        conn.send_hello(Timeout::Infinite)?;

        conn.send
            .send_message_write_all(&standard_messages::add_match(
                "type='signal',sender='rs.wl-gammarelay',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
            ))?;

        let mut temperature = 6500;
        let mut gamma = 1.0;
        let mut brightness = 1.0;

        if server_running {
            let mut t_done = false;
            let mut g_done = false;
            let mut b_done = false;
            let mut msg = MessageBuilder::new()
                .call("Get")
                .on("/")
                .with_interface("org.freedesktop.DBus.Properties")
                .at("rs.wl-gammarelay")
                .build();

            msg.body.reset();
            msg.body.push_param("rs.wl.gammarelay")?;
            msg.body.push_param("Temperature")?;
            let t_serial = conn.send.send_message_write_all(&msg)?;
            msg.body.reset();
            msg.body.push_param("rs.wl.gammarelay")?;
            msg.body.push_param("Gamma")?;
            let g_serial = conn.send.send_message_write_all(&msg)?;
            msg.body.reset();
            msg.body.push_param("rs.wl.gammarelay")?;
            msg.body.push_param("Brightness")?;
            let b_serial = conn.send.send_message_write_all(&msg)?;

            while !(t_done && g_done && b_done) {
                let msg = conn.recv.get_next_message(Timeout::Infinite)?;
                let mut parser = msg.body.parser();
                if msg.dynheader.response_serial == Some(t_serial) {
                    temperature = parser.get::<UnVariant>()?.get::<u16>()?;
                    t_done = true;
                } else if msg.dynheader.response_serial == Some(g_serial) {
                    gamma = parser.get::<UnVariant>()?.get::<f64>()?;
                    g_done = true;
                } else if msg.dynheader.response_serial == Some(b_serial) {
                    brightness = parser.get::<UnVariant>()?.get::<f64>()?;
                    b_done = true;
                }
            }
        }

        let mut this = Self {
            format,
            conn,
            temperature,
            gamma,
            brightness,
            prev_output: None,
        };

        this.print();

        Ok(this)
    }

    pub fn run(&mut self, blocking: bool) -> Result<()> {
        let timeout = if blocking {
            Timeout::Infinite
        } else {
            Timeout::Nonblock
        };
        loop {
            let msg = match self.conn.recv.get_next_message(timeout) {
                Ok(msg) => msg,
                Err(rustbus::connection::Error::TimedOut) => return Ok(()),
                Err(e) => return Err(e.into()),
            };

            if msg.typ == MessageType::Signal
                && msg.dynheader.interface.as_deref() == Some("org.freedesktop.DBus.Properties")
                && msg.dynheader.member.as_deref() == Some("PropertiesChanged")
            {
                let mut parser = msg.body.parser();
                let iface = parser.get::<&str>()?;
                if iface == "rs.wl.gammarelay" {
                    let changed = parser.get::<HashMap<&str, UnVariant>>()?;
                    let invalidated = parser.get::<Vec<&str>>()?;
                    assert!(invalidated.is_empty());
                    if let Some(v) = changed.get("Temperature") {
                        self.temperature = v.get::<u16>()?;
                    }
                    if let Some(v) = changed.get("Gamma") {
                        self.gamma = v.get::<f64>()?;
                    }
                    if let Some(v) = changed.get("Brightness") {
                        self.brightness = v.get::<f64>()?;
                    }
                    self.print();
                }
            }
        }
    }

    fn print(&mut self) {
        let output = self
            .format
            .replace("{t}", &self.temperature.to_string())
            .replace("{g}", &format!("{:.2}", self.gamma))
            .replace("{b}", &format!("{:.2}", self.brightness))
            .replace("{bp}", &format!("{:.0}", self.brightness * 100.));
        if self.prev_output.as_ref().is_none_or(|prev| *prev != output) {
            println!("{output}");
            self.prev_output = Some(output);
        }
    }
}
