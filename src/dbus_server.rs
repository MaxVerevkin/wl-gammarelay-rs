use crate::color::Color;
use crate::wayland::Request;
use anyhow::Result;
use tokio::sync::mpsc;
use zbus::{dbus_interface, Connection};

#[derive(Debug)]
pub struct Server {
    tx: mpsc::Sender<Request>,
    color: Color,
    output_name: Option<String>,
}

pub async fn run(tx: mpsc::Sender<Request>) -> Result<Option<Connection>> {
    let mut builder = zbus::ConnectionBuilder::session()?;
    builder = builder.serve_at("/", new_server(tx, None))?;
    builder = builder.name("rs.wl-gammarelay")?;
    let session = builder.build().await;
    match session {
        Err(zbus::Error::NameTaken) => Ok(None),
        Err(e) => Err(e.into()),
        Ok(server) => Ok(Some(server)),
    }
}

impl Server {
    async fn send_color(&self) {
        let _ = self
            .tx
            .send(Request::SetColor {
                color: self.color,
                output_name: self.output_name.clone(),
            })
            .await;
    }
}

pub fn new_server(tx: mpsc::Sender<Request>, output_name: Option<String>) -> Server {
    Server {
        tx,
        output_name,
        color: Default::default(),
    }
}

#[dbus_interface(name = "rs.wl.gammarelay")]
impl Server {
    #[dbus_interface(property)]
    async fn temperature(&self) -> u16 {
        self.color.temp
    }

    #[dbus_interface(property)]
    async fn set_temperature(&mut self, temp: u16) -> Result<(), zbus::fdo::Error> {
        if (1000..=10000).contains(&temp) {
            self.color.temp = temp;
            self.send_color().await;
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "temperature must be in range [1000,10000]".into(),
            ))
        }
    }

    async fn update_temperature(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
        delta_temp: i16,
    ) -> Result<(), zbus::fdo::Error> {
        self.color.temp = (self.color.temp as i16 + delta_temp).clamp(1_000, 10_000) as _;
        self.send_color().await;
        self.temperature_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn gamma(&self) -> f64 {
        self.color.gamma
    }

    #[dbus_interface(property)]
    async fn set_gamma(&mut self, gamma: f64) -> Result<(), zbus::fdo::Error> {
        if gamma > 0.0 {
            self.color.gamma = gamma;
            self.send_color().await;
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "gamma must be greater than zero".into(),
            ))
        }
    }

    async fn update_gamma(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
        delta_gamma: f64,
    ) -> Result<(), zbus::fdo::Error> {
        self.color.gamma = (self.color.gamma + delta_gamma).max(0.0);
        self.send_color().await;
        self.gamma_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn brightness(&self) -> f64 {
        self.color.brightness
    }

    #[dbus_interface(property)]
    async fn set_brightness(&mut self, brightness: f64) -> Result<(), zbus::fdo::Error> {
        if (0.0..=1.0).contains(&brightness) {
            self.color.brightness = brightness;
            self.send_color().await;
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "brightness must be in range [0,1]".into(),
            ))
        }
    }

    async fn update_brightness(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
        delta_brightness: f64,
    ) -> Result<(), zbus::fdo::Error> {
        self.color.brightness = (self.color.brightness + delta_brightness).clamp(0.0, 1.0);
        self.send_color().await;
        self.brightness_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn inverted(&self) -> bool {
        self.color.inverted
    }

    #[dbus_interface(property)]
    async fn set_inverted(&mut self, value: bool) {
        self.color.inverted = value;
        self.send_color().await;
    }

    async fn toggle_inverted(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        self.color.inverted = !self.color.inverted;
        self.send_color().await;
        self.brightness_changed(&cx).await?;
        Ok(())
    }
}
