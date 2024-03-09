use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};

use crate::State;
use anyhow::Result;
use rustbus::{
    connection::Timeout,
    get_session_bus_path,
    message_builder::MarshalledMessage,
    params::{Param, Variant},
    wire::unmarshal::traits::Variant as UnVariant,
    DuplexConn, MessageBuilder,
};
use rustbus_service::rustbus;
use rustbus_service::{Access, InterfaceImp, MethodContext, PropContext, Service};

pub struct DbusServer {
    conn: DuplexConn,
    service: Service<State>,
}

impl AsRawFd for DbusServer {
    fn as_raw_fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }
}

impl DbusServer {
    pub fn new() -> Result<Option<Self>> {
        let mut conn = rustbus::DuplexConn::connect_to_bus(get_session_bus_path()?, true)?;
        conn.send_hello(Timeout::Infinite)?;

        let mut service = Service::new();

        let req_name_serial =
            conn.send
                .send_message_write_all(&rustbus::standard_messages::request_name(
                    "rs.wl-gammarelay",
                    0,
                ))?;
        let req_name_resp = service.get_reply(&mut conn, req_name_serial, Timeout::Infinite)?;
        if req_name_resp.body.parser().get::<u32>()?
            != rustbus::standard_messages::DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER
        {
            return Ok(None);
        }

        let gammarelay_iface = InterfaceImp::new("rs.wl.gammarelay")
            .with_method::<(), ()>("ToggleInverted", toggle_inverted_root_cb)
            .with_method::<UpdateTemperatureArgs, ()>(
                "UpdateTemperature",
                update_temperature_root_cb,
            )
            .with_method::<UpdateGammaArgs, ()>("UpdateGamma", update_gamma_root_cb)
            .with_method::<UpdateBrightnessArgs, ()>("UpdateBrightness", update_brightness_root_cb)
            .with_prop(
                "Inverted",
                Access::ReadWrite(get_inverted_root_cb, set_inverted_root_cb),
            )
            .with_prop(
                "Temperature",
                Access::ReadWrite(get_temperature_root_cb, set_temperature_root_cb),
            )
            .with_prop(
                "Gamma",
                Access::ReadWrite(get_gamma_root_cb, set_gamma_root_cb),
            )
            .with_prop(
                "Brightness",
                Access::ReadWrite(get_brightness_root_cb, set_brightness_root_cb),
            );

        service.root_mut().add_interface(gammarelay_iface);

        // TODO: add other interfaces with service.get_object_mut("/outputs/[id]").add_interface()

        Ok(Some(Self { conn, service }))
    }

    pub fn poll(&mut self, state: &mut State) -> Result<()> {
        self.service.run(&mut self.conn, state, Timeout::Nonblock)?;
        Ok(())
    }
}

fn toggle_inverted_root_cb(ctx: &mut MethodContext<State>, _args: ()) {
    let inverted = !ctx.state.color().inverted;
    ctx.state.set_inverted(inverted);

    let sig = prop_changed_message(
        ctx.object_path,
        "rs.wl.gammarelay",
        "Inverted",
        ctx.state.color().inverted.into(),
    );
    ctx.conn.send.send_message_write_all(&sig).unwrap();
}

fn get_inverted_root_cb(ctx: PropContext<State>) -> bool {
    ctx.state.color().inverted
}

fn set_inverted_root_cb(ctx: PropContext<State>, val: UnVariant) {
    let val = val.get::<bool>().unwrap();
    if ctx.state.color().inverted != val {
        ctx.state.set_inverted(val);

        let sig = prop_changed_message(ctx.object_path, "rs.wl.gammarelay", ctx.name, val.into());
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

#[derive(rustbus_service::Args)]
struct UpdateBrightnessArgs {
    delta: f64,
}

fn update_brightness_root_cb(ctx: &mut MethodContext<State>, args: UpdateBrightnessArgs) {
    let val = (ctx.state.color().brightness + args.delta).clamp(0.0, 1.0);

    if ctx.state.color().brightness != val {
        ctx.state.set_brightness(val);

        let sig = prop_changed_message(
            ctx.object_path,
            "rs.wl.gammarelay",
            "Brightness",
            val.into(),
        );
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

fn get_brightness_root_cb(ctx: PropContext<State>) -> f64 {
    ctx.state.color().brightness
}

fn set_brightness_root_cb(ctx: PropContext<State>, val: UnVariant) {
    let val = val.get::<f64>().unwrap().clamp(0.0, 1.0);

    if ctx.state.color().brightness != val {
        ctx.state.set_brightness(val);

        let sig = prop_changed_message(ctx.object_path, "rs.wl.gammarelay", ctx.name, val.into());
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

#[derive(rustbus_service::Args)]
struct UpdateTemperatureArgs {
    delta: i16,
}

fn update_temperature_root_cb(ctx: &mut MethodContext<State>, args: UpdateTemperatureArgs) {
    let val = (ctx.state.color().temp as i16 + args.delta).clamp(1_000, 10_000) as u16;

    if ctx.state.color().temp != val {
        ctx.state.set_temperature(val);

        let sig = prop_changed_message(
            ctx.object_path,
            "rs.wl.gammarelay",
            "Temperature",
            val.into(),
        );
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

fn get_temperature_root_cb(ctx: PropContext<State>) -> u16 {
    ctx.state.color().temp
}

fn set_temperature_root_cb(ctx: PropContext<State>, val: UnVariant) {
    let val = val.get::<u16>().unwrap().clamp(1_000, 10_000);
    if ctx.state.color().temp != val {
        ctx.state.set_temperature(val);

        let sig = prop_changed_message(ctx.object_path, "rs.wl.gammarelay", ctx.name, val.into());
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

#[derive(rustbus_service::Args)]
struct UpdateGammaArgs {
    delta: f64,
}

fn update_gamma_root_cb(ctx: &mut MethodContext<State>, args: UpdateGammaArgs) {
    let val = (ctx.state.color().gamma + args.delta).max(0.1);

    if ctx.state.color().gamma != val {
        ctx.state.set_gamma(val);

        let sig = prop_changed_message(ctx.object_path, "rs.wl.gammarelay", "Gamma", val.into());
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

fn get_gamma_root_cb(ctx: PropContext<State>) -> f64 {
    ctx.state.color().gamma
}

fn set_gamma_root_cb(ctx: PropContext<State>, val: UnVariant) {
    let val = val.get::<f64>().unwrap().max(0.1);
    if ctx.state.color().gamma != val {
        ctx.state.set_gamma(val);

        let sig = prop_changed_message(ctx.object_path, "rs.wl.gammarelay", ctx.name, val.into());
        ctx.conn.send.send_message_write_all(&sig).unwrap();
    }
}

fn prop_changed_message(path: &str, iface: &str, prop: &str, value: Param) -> MarshalledMessage {
    let mut map = HashMap::new();
    map.insert(
        prop,
        Variant {
            sig: value.sig(),
            value,
        },
    );

    let mut sig = MessageBuilder::new()
        .signal("org.freedesktop.DBus.Properties", "PropertiesChanged", path)
        .build();
    sig.body.push_param(iface).unwrap();
    sig.body.push_param(map).unwrap();
    sig.body.push_param::<&[&str]>(&[]).unwrap();
    sig
}
