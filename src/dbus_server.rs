use std::sync::Arc;

use crate::color::Color;
use crate::wayland::Request;
use anyhow::Result;
use tokio::sync::{mpsc, Mutex};
use zbus::{dbus_interface, Connection};

#[derive(Debug, Default, Clone)]
pub struct RootServer {
    outputs: Arc<Mutex<Vec<Arc<Mutex<Output>>>>>,
}

#[derive(Debug)]
struct Server(Arc<Mutex<Output>>);

#[derive(Debug)]
struct Output {
    tx: mpsc::Sender<Request>,
    color: Color,
    output_name: String,
}

pub async fn run() -> Result<Option<(Connection, RootServer)>> {
    let mut builder = zbus::ConnectionBuilder::session()?;
    let root_server = RootServer::default();
    builder = builder.serve_at("/", root_server.clone())?;
    builder = builder.name("rs.wl-gammarelay")?;
    let session = builder.build().await;
    match session {
        Err(zbus::Error::NameTaken) => Ok(None),
        Err(e) => Err(e.into()),
        Ok(server) => Ok(Some((server, root_server))),
    }
}

impl Output {
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

#[dbus_interface(name = "rs.wl.gammarelay")]
impl Server {
    #[dbus_interface(property)]
    async fn temperature(&self) -> u16 {
        let output = self.0.lock().await;
        output.color.temp
    }

    #[dbus_interface(property)]
    async fn set_temperature(&mut self, temp: u16) -> Result<(), zbus::fdo::Error> {
        if (1000..=10000).contains(&temp) {
            let mut output = self.0.lock().await;
            output.color.temp = temp;
            output.send_color().await;
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
        {
            let mut output = self.0.lock().await;
            output.color.temp = (output.color.temp as i16 + delta_temp).clamp(1_000, 10_000) as _;
            output.send_color().await;
        }
        self.temperature_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn gamma(&self) -> f64 {
        let output = self.0.lock().await;
        output.color.gamma
    }

    #[dbus_interface(property)]
    async fn set_gamma(&mut self, gamma: f64) -> Result<(), zbus::fdo::Error> {
        if gamma > 0.0 {
            let mut output = self.0.lock().await;
            output.color.gamma = gamma;
            output.send_color().await;
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
        {
            let mut output = self.0.lock().await;
            output.color.gamma = (output.color.gamma + delta_gamma).max(0.0);
            output.send_color().await;
        }
        self.gamma_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn brightness(&self) -> f64 {
        let output = self.0.lock().await;
        output.color.brightness
    }

    #[dbus_interface(property)]
    async fn set_brightness(&mut self, brightness: f64) -> Result<(), zbus::fdo::Error> {
        if (0.0..=1.0).contains(&brightness) {
            let mut output = self.0.lock().await;
            output.color.brightness = brightness;
            output.send_color().await;
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
        {
            let mut output = self.0.lock().await;
            output.color.brightness = (output.color.brightness + delta_brightness).clamp(0.0, 1.0);
            output.send_color().await;
        }
        self.brightness_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn inverted(&self) -> bool {
        let output = self.0.lock().await;
        output.color.inverted
    }

    #[dbus_interface(property)]
    async fn set_inverted(&mut self, value: bool) {
        let mut output = self.0.lock().await;
        output.color.inverted = value;
        output.send_color().await;
    }

    async fn toggle_inverted(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        {
            let mut output = self.0.lock().await;
            output.color.inverted = !output.color.inverted;
            output.send_color().await;
        }
        self.brightness_changed(&cx).await?;
        Ok(())
    }
}

impl RootServer {
    pub async fn add_output(
        &self,
        instance: &mut Connection,
        tx: mpsc::Sender<Request>,
        output_name: String,
    ) -> Result<()> {
        let path = format!("/outputs/{}", output_name.replace('-', "_"));
        let output = Output {
            tx,
            output_name,
            color: Default::default(),
        };
        let server = Server(Arc::new(Mutex::new(output)));
        self.outputs.lock().await.push(server.0.clone());
        instance.object_server().at(path, server).await?;

        Ok(())
    }

    pub async fn remove_output(
        &self,
        instance: &mut Connection,
        output_name: String,
    ) -> Result<()> {
        let mut outputs = self.outputs.lock().await;
        let mut output_index = None;
        for (index, output) in outputs.iter().enumerate() {
            if output.lock().await.output_name == output_name {
                output_index = Some(index);
                break;
            }
        }
        if let Some(index) = output_index {
            outputs.remove(index);
            let path = format!("/outputs/{}", output_name.replace('-', "_"));
            instance.object_server().remove::<Server, _>(path).await?;
        }

        Ok(())
    }
}

#[dbus_interface(name = "rs.wl.gammarelay")]
impl RootServer {
    #[dbus_interface(property)]
    async fn temperature(&self) -> u16 {
        let outputs = self.outputs.lock().await;
        let mut temp_sum = 0;
        for output in outputs.iter() {
            let output = output.lock().await;
            temp_sum += output.color.temp;
        }
        temp_sum / outputs.len() as u16
    }

    #[dbus_interface(property)]
    async fn set_temperature(&mut self, temp: u16) -> Result<(), zbus::fdo::Error> {
        if (1000..=10000).contains(&temp) {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.temp = temp;
                output.send_color().await;
            }
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
        {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.temp =
                    (output.color.temp as i16 + delta_temp).clamp(1_000, 10_000) as _;
                output.send_color().await;
            }
        }
        self.temperature_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn gamma(&self) -> f64 {
        let outputs = self.outputs.lock().await;
        let mut gamma_sum = 0.0;
        for output in outputs.iter() {
            let output = output.lock().await;
            gamma_sum += output.color.gamma;
        }
        gamma_sum / outputs.len() as f64
    }

    #[dbus_interface(property)]
    async fn set_gamma(&mut self, gamma: f64) -> Result<(), zbus::fdo::Error> {
        if gamma > 0.0 {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.gamma = gamma;
                output.send_color().await;
            }
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
        {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.gamma = (output.color.gamma + delta_gamma).max(0.0);
                output.send_color().await;
            }
        }
        self.gamma_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn brightness(&self) -> f64 {
        let outputs = self.outputs.lock().await;
        let mut brightness_sum = 0.0;
        for output in outputs.iter() {
            let output = output.lock().await;
            brightness_sum += output.color.brightness;
        }
        brightness_sum / outputs.len() as f64
    }

    #[dbus_interface(property)]
    async fn set_brightness(&mut self, brightness: f64) -> Result<(), zbus::fdo::Error> {
        if (0.0..=1.0).contains(&brightness) {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.brightness = brightness;
                output.send_color().await;
            }
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
        {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.brightness =
                    (output.color.brightness + delta_brightness).clamp(0.0, 1.0);
                output.send_color().await;
            }
        }
        self.brightness_changed(&cx).await?;
        Ok(())
    }

    #[dbus_interface(property)]
    async fn inverted(&self) -> bool {
        let outputs = self.outputs.lock().await;
        let mut all_inverted = true;
        for output in outputs.iter() {
            let output = output.lock().await;
            all_inverted &= output.color.inverted;
            if !all_inverted {
                break;
            }
        }
        all_inverted
    }

    #[dbus_interface(property)]
    async fn set_inverted(&mut self, value: bool) {
        let outputs = self.outputs.lock().await;
        for output in outputs.iter() {
            let mut output = output.lock().await;
            output.color.inverted = value;
            output.send_color().await;
        }
    }

    async fn toggle_inverted(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
    ) -> Result<(), zbus::fdo::Error> {
        let inverted = self.inverted().await;
        {
            let outputs = self.outputs.lock().await;
            for output in outputs.iter() {
                let mut output = output.lock().await;
                output.color.inverted = !inverted;
                output.send_color().await;
            }
        }
        self.brightness_changed(&cx).await?;
        Ok(())
    }
}
