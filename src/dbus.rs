use crate::Color;
use anyhow::Result;
use tokio::sync::mpsc;
use zbus::dbus_interface;

pub struct Server {
    tx: mpsc::Sender<Request>,
    color: Color,
}

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
            let _ = self.tx.send(Request::SetColor(self.color)).await;
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
            let _ = self.tx.send(Request::SetColor(self.color)).await;
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "brightness must be in range [0,1]".into(),
            ))
        }
    }
}
