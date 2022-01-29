use anyhow::Result;
use futures::stream::StreamExt;
use zbus::dbus_proxy;

pub async fn watch_dbus(format: &str) -> Result<()> {
    let zbus::Address::Unix(addr) = zbus::Address::session()?;
    let stream = tokio::net::UnixStream::connect(addr).await?;
    let conn = zbus::ConnectionBuilder::socket(stream)
        .internal_executor(false)
        .build()
        .await?;
    {
        let conn = conn.clone();
        tokio::spawn(async move {
            loop {
                conn.executor().tick().await;
            }
        });
    }
    let proxy = BusInterfaceProxy::new(&conn).await?;
    let mut temperature = proxy.temperature().await?;
    let mut brightness = proxy.brightness().await?;
    let mut t_stream = proxy.receive_temperature_changed().await;
    let mut b_stream = proxy.receive_brightness_changed().await;
    loop {
        print_formatted(format, temperature, brightness);
        tokio::select! {
            Some(t) = t_stream.next() => {
                temperature = t.get().await?;
            }
            Some(b) = b_stream.next() => {
                brightness = b.get().await?;
            }
        }
    }
}

fn print_formatted(format: &str, temperature: u16, brightness: f64) {
    println!(
        "{}",
        format
            .replace("{t}", &temperature.to_string())
            .replace("{b}", &format!("{:.2}", brightness))
            .replace("{bp}", &format!("{:.0}", brightness * 100.))
    );
}

#[dbus_proxy(
    interface = "rs.wl.gammarelay",
    default_service = "rs.wl-gammarelay",
    default_path = "/"
)]
trait BusInterface {
    /// UpdateBrightness method
    fn update_brightness(&self, delta_brightness: f64) -> zbus::Result<()>;

    /// UpdateTemperature method
    fn update_temperature(&self, delta_temp: i16) -> zbus::Result<()>;

    /// Brightness property
    #[dbus_proxy(property)]
    fn brightness(&self) -> zbus::Result<f64>;
    #[dbus_proxy(property)]
    fn set_brightness(&self, value: f64) -> zbus::Result<()>;

    /// Temperature property
    #[dbus_proxy(property)]
    fn temperature(&self) -> zbus::Result<u16>;
    #[dbus_proxy(property)]
    fn set_temperature(&self, value: u16) -> zbus::Result<()>;
}
