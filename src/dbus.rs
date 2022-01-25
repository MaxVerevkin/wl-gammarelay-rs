use crate::Color;
use anyhow::Result;
use tokio::sync::mpsc;
use zbus::dbus_interface;

#[derive(Debug)]
pub struct Server {
    tx: mpsc::Sender<Request>,
    color: Color,
}

#[derive(Debug)]
pub enum Request {
    SetColor(Color),
}

impl Server {
    pub fn new(tx: mpsc::Sender<Request>) -> Self {
        Self {
            tx,
            color: Default::default(),
        }
    }

    pub async fn run(self) -> Result<()> {
        let stream = match zbus::Address::session().unwrap() {
            zbus::Address::Unix(s) => tokio::net::UnixStream::connect(s).await?,
        };
        let conn = zbus::ConnectionBuilder::socket(stream)
            .internal_executor(false)
            .serve_at("/", self)?
            .name("rs.wl-gammarelay")?
            .build()
            .await?;
        tokio::spawn(async move {
            loop {
                conn.executor().tick().await;
            }
        });
        Ok(())
    }

    async fn send_color(&self) {
        let _ = self.tx.send(Request::SetColor(self.color)).await;
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

    async fn update_brightness(
        &mut self,
        #[zbus(signal_context)] cx: zbus::SignalContext<'_>,
        delta_brightness: f64,
    ) -> Result<(), zbus::fdo::Error> {
        self.color.brightness = (self.color.brightness + delta_brightness).clamp(0.0, 1.0) as _;
        self.send_color().await;
        self.temperature_changed(&cx).await?;
        Ok(())
    }
}
