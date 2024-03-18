use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};

use crate::color::Color;
use crate::WaylandState;
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
    service: Service<WaylandState>,
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

        let gammarelay_root_iface = InterfaceImp::new("rs.wl.gammarelay")
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

        let root = service.root_mut();
        root.add_interface(gammarelay_root_iface);
        root.add_child("outputs", rustbus_service::Object::new());

        Ok(Some(Self { conn, service }))
    }

    pub fn add_output(&mut self, reg_name: u32, name: &str) {
        let toggle_inverted_output_cb = move |ctx: &mut MethodContext<WaylandState>, _args: ()| {
            let global_color = ctx.state.color();

            let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
            let color = output.color();
            let inverted = !color.inverted;
            output.set_color(Color { inverted, ..color });

            let value = inverted.into();
            signal_change(&mut ctx.conn.send, ctx.object_path, "Inverted", value);

            if ctx.state.color().inverted != global_color.inverted {
                let value = inverted.into();
                signal_change(&mut ctx.conn.send, "/", "Inverted", value);
            }
        };

        let get_inverted_output_cb = move |ctx: PropContext<WaylandState>| {
            ctx.state
                .output_by_reg_name(reg_name)
                .unwrap()
                .color()
                .inverted
        };

        let set_inverted_output_cb = move |ctx: PropContext<WaylandState>, val: UnVariant| {
            let global_color = ctx.state.color();

            let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
            let color = output.color();
            let inverted = val.get::<bool>().unwrap();

            if color.inverted != inverted {
                output.set_color(Color { inverted, ..color });

                let value = inverted.into();
                signal_change(&mut ctx.conn.send, ctx.object_path, "Inverted", value);

                if ctx.state.color().inverted != global_color.inverted {
                    let value = inverted.into();
                    signal_change(&mut ctx.conn.send, "/", "Inverted", value);
                }
            }
        };

        let update_brightness_output_cb =
            move |ctx: &mut MethodContext<WaylandState>, args: UpdateBrightnessArgs| {
                let global_color = ctx.state.color();

                let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
                let color = output.color();
                let brightness = (color.brightness + args.delta).clamp(0.0, 1.0);

                if color.brightness != brightness {
                    output.set_color(Color {
                        brightness,
                        ..color
                    });

                    let value = brightness.into();
                    signal_change(&mut ctx.conn.send, ctx.object_path, "Brightness", value);

                    let brightness = ctx.state.color().brightness;
                    if brightness != global_color.brightness {
                        let value = brightness.into();
                        signal_change(&mut ctx.conn.send, "/", "Brightness", value);
                    }
                }
            };

        let get_brightness_output_cb = move |ctx: PropContext<WaylandState>| {
            ctx.state
                .output_by_reg_name(reg_name)
                .unwrap()
                .color()
                .brightness
        };

        let set_brightness_output_cb = move |ctx: PropContext<WaylandState>, val: UnVariant| {
            let global_color = ctx.state.color();

            let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
            let color = output.color();
            let brightness = val.get::<f64>().unwrap().clamp(0.0, 1.0);

            if color.brightness != brightness {
                output.set_color(Color {
                    brightness,
                    ..color
                });

                let value = brightness.into();
                signal_change(&mut ctx.conn.send, ctx.object_path, "Brightness", value);

                let brightness = ctx.state.color().brightness;
                if brightness != global_color.brightness {
                    let value = brightness.into();
                    signal_change(&mut ctx.conn.send, "/", "Brightness", value);
                }
            }
        };

        let update_temperature_output_cb =
            move |ctx: &mut MethodContext<WaylandState>, args: UpdateTemperatureArgs| {
                let global_color = ctx.state.color();

                let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
                if let Some(new_color) = output.color().with_updated_temp(args.delta) {
                    output.set_color(new_color);

                    let value = new_color.temp.into();
                    signal_change(&mut ctx.conn.send, ctx.object_path, "Temperature", value);

                    let temp = ctx.state.color().temp;
                    if temp != global_color.temp {
                        let value = temp.into();
                        signal_change(&mut ctx.conn.send, "/", "Temperature", value);
                    }
                }
            };

        let get_temperature_output_cb = move |ctx: PropContext<WaylandState>| {
            ctx.state.output_by_reg_name(reg_name).unwrap().color().temp
        };

        let set_temperature_output_cb = move |ctx: PropContext<WaylandState>, val: UnVariant| {
            let global_color = ctx.state.color();

            let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
            let color = output.color();
            let temp = val.get::<u16>().unwrap().clamp(1_000, 10_000);

            if color.temp != temp {
                output.set_color(Color { temp, ..color });

                let value = temp.into();
                signal_change(&mut ctx.conn.send, ctx.object_path, "Temperature", value);

                let temp = ctx.state.color().temp;
                if temp != global_color.temp {
                    let value = temp.into();
                    signal_change(&mut ctx.conn.send, "/", "Temperature", value);
                }
            }
        };

        let update_gamma_output_cb =
            move |ctx: &mut MethodContext<WaylandState>, args: UpdateGammaArgs| {
                let global_color = ctx.state.color();

                let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
                let color = output.color();
                let gamma = (color.gamma + args.delta).max(0.1);

                if color.gamma != gamma {
                    output.set_color(Color { gamma, ..color });

                    let value = gamma.into();
                    signal_change(&mut ctx.conn.send, ctx.object_path, "Gamma", value);

                    let gamma = ctx.state.color().gamma;
                    if gamma != global_color.gamma {
                        let value = gamma.into();
                        signal_change(&mut ctx.conn.send, "/", "Gamma", value);
                    }
                }
            };

        let get_gamma_output_cb = move |ctx: PropContext<WaylandState>| {
            ctx.state
                .output_by_reg_name(reg_name)
                .unwrap()
                .color()
                .gamma
        };

        let set_gamma_output_cb = move |ctx: PropContext<WaylandState>, val: UnVariant| {
            let global_color = ctx.state.color();

            let output = ctx.state.mut_output_by_reg_name(reg_name).unwrap();
            let color = output.color();
            let gamma = val.get::<f64>().unwrap().max(0.1);

            if color.gamma != gamma {
                output.set_color(Color { gamma, ..color });

                let value = gamma.into();
                signal_change(&mut ctx.conn.send, ctx.object_path, "Gamma", value);

                let gamma = ctx.state.color().gamma;
                if gamma != global_color.gamma {
                    let value = gamma.into();
                    signal_change(&mut ctx.conn.send, "/", "Gamma", value);
                }
            }
        };

        let gammarelay_output_iface = InterfaceImp::new("rs.wl.gammarelay")
            .with_method::<(), ()>("ToggleInverted", toggle_inverted_output_cb)
            .with_method::<UpdateTemperatureArgs, ()>(
                "UpdateTemperature",
                update_temperature_output_cb,
            )
            .with_method::<UpdateGammaArgs, ()>("UpdateGamma", update_gamma_output_cb)
            .with_method::<UpdateBrightnessArgs, ()>(
                "UpdateBrightness",
                update_brightness_output_cb,
            )
            .with_prop(
                "Inverted",
                Access::ReadWrite(get_inverted_output_cb, set_inverted_output_cb),
            )
            .with_prop(
                "Temperature",
                Access::ReadWrite(get_temperature_output_cb, set_temperature_output_cb),
            )
            .with_prop(
                "Gamma",
                Access::ReadWrite(get_gamma_output_cb, set_gamma_output_cb),
            )
            .with_prop(
                "Brightness",
                Access::ReadWrite(get_brightness_output_cb, set_brightness_output_cb),
            );

        let mut object = rustbus_service::Object::new();
        object.add_interface(gammarelay_output_iface);

        let outputs_object = self
            .service
            .get_object_mut("/outputs")
            .expect("object /outputs not found");
        outputs_object.add_child(name.replace('-', "_"), object);
    }

    pub fn remove_output(&mut self, name: &str) {
        let outputs_object = self
            .service
            .get_object_mut("/outputs")
            .expect("object /outputs not found");

        outputs_object.remove_child(&name.replace('-', "_"));
    }

    pub fn poll(&mut self, state: &mut WaylandState) -> Result<()> {
        self.service.run(&mut self.conn, state, Timeout::Nonblock)?;
        Ok(())
    }
}

fn toggle_inverted_root_cb(ctx: &mut MethodContext<WaylandState>, _args: ()) {
    let inverted = !ctx.state.color().inverted;
    ctx.state.set_inverted(inverted);

    signal_change(
        &mut ctx.conn.send,
        ctx.object_path,
        "Inverted",
        inverted.into(),
    );
    signal_updated_property_to_outputs(ctx, "Inverted", inverted.into());
}

fn get_inverted_root_cb(ctx: PropContext<WaylandState>) -> bool {
    ctx.state.color().inverted
}

fn set_inverted_root_cb(ctx: PropContext<WaylandState>, val: UnVariant) {
    let val = val.get::<bool>().unwrap();
    if ctx.state.color().inverted != val {
        ctx.state.set_inverted(val);

        signal_change(&mut ctx.conn.send, ctx.object_path, ctx.name, val.into());
        signal_set_property_to_outputs(ctx, val.into());
    }
}

#[derive(rustbus_service::Args)]
struct UpdateBrightnessArgs {
    delta: f64,
}

fn update_brightness_root_cb(ctx: &mut MethodContext<WaylandState>, args: UpdateBrightnessArgs) {
    if ctx.state.update_brightness(args.delta) {
        let val = ctx.state.color().brightness;
        signal_change(
            &mut ctx.conn.send,
            ctx.object_path,
            "Brightness",
            val.into(),
        );
        signal_updated_property_to_outputs(ctx, "Brightness", val.into());
    }
}

fn get_brightness_root_cb(ctx: PropContext<WaylandState>) -> f64 {
    ctx.state.color().brightness
}

fn set_brightness_root_cb(ctx: PropContext<WaylandState>, val: UnVariant) {
    let val = val.get::<f64>().unwrap().clamp(0.0, 1.0);
    if ctx.state.color().brightness != val {
        ctx.state.set_brightness(val);

        signal_change(&mut ctx.conn.send, ctx.object_path, ctx.name, val.into());
        signal_set_property_to_outputs(ctx, val.into());
    }
}

#[derive(rustbus_service::Args)]
struct UpdateTemperatureArgs {
    delta: i16,
}

fn update_temperature_root_cb(ctx: &mut MethodContext<WaylandState>, args: UpdateTemperatureArgs) {
    if ctx.state.update_temperature(args.delta) {
        let val = ctx.state.color().temp;
        signal_change(
            &mut ctx.conn.send,
            ctx.object_path,
            "Temperature",
            val.into(),
        );
        signal_updated_property_to_outputs(ctx, "Temperature", val.into());
    }
}

fn get_temperature_root_cb(ctx: PropContext<WaylandState>) -> u16 {
    ctx.state.color().temp
}

fn set_temperature_root_cb(ctx: PropContext<WaylandState>, val: UnVariant) {
    let val = val.get::<u16>().unwrap().clamp(1_000, 10_000);
    if ctx.state.color().temp != val {
        ctx.state.set_temperature(val);

        signal_change(&mut ctx.conn.send, ctx.object_path, ctx.name, val.into());
        signal_set_property_to_outputs(ctx, val.into());
    }
}

#[derive(rustbus_service::Args)]
struct UpdateGammaArgs {
    delta: f64,
}

fn update_gamma_root_cb(ctx: &mut MethodContext<WaylandState>, args: UpdateGammaArgs) {
    if ctx.state.update_gamma(args.delta) {
        let val = ctx.state.color().gamma;
        signal_change(&mut ctx.conn.send, ctx.object_path, "Gamma", val.into());
        signal_updated_property_to_outputs(ctx, "Gamma", val.into());
    }
}

fn get_gamma_root_cb(ctx: PropContext<WaylandState>) -> f64 {
    ctx.state.color().gamma
}

fn set_gamma_root_cb(ctx: PropContext<WaylandState>, val: UnVariant) {
    let val = val.get::<f64>().unwrap().max(0.1);
    if ctx.state.color().gamma != val {
        ctx.state.set_gamma(val);

        signal_change(&mut ctx.conn.send, ctx.object_path, ctx.name, val.into());
        signal_set_property_to_outputs(ctx, val.into());
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

fn signal_change(send: &mut rustbus::SendConn, path: &str, prop: &str, value: Param) {
    let output_sig = prop_changed_message(path, "rs.wl.gammarelay", prop, value);
    send.send_message_write_all(&output_sig).unwrap();
}

fn signal_set_property_to_outputs(ctx: PropContext<WaylandState>, value: Param) {
    for output in ctx
        .state
        .outputs
        .iter()
        .filter(|output| output.color_changed())
    {
        if let Some(path) = output.object_path() {
            signal_change(&mut ctx.conn.send, &path, ctx.name, value.clone());
        }
    }
}

fn signal_updated_property_to_outputs(
    ctx: &mut MethodContext<WaylandState>,
    name: &str,
    value: Param,
) {
    for output in ctx
        .state
        .outputs
        .iter()
        .filter(|output| output.color_changed())
    {
        if let Some(path) = output.object_path() {
            signal_change(&mut ctx.conn.send, &path, name, value.clone());
        }
    }
}
