use actix;
use gst;
use gst::prelude::*;

use proto;

use std::collections::HashMap;
use std::net::Ipv4Addr;

pub enum PlayerControl {
    Gain(f64, f64),
    Enable(bool),
    Stream {
        autostart: bool,
        threshold: u32,
        output_threshold: u64,
        replay_gain: f64,
        server_port: u16,
        server_ip: Ipv4Addr,
        control_ip: Ipv4Addr,
        http_headers: String,
    },
    Stop,
    Pause,
    Unpause,
}

impl actix::Message for PlayerControl {
    type Result = ();
}

pub enum PlayerMessages {
    Flushed,
    Paused,
    Unpaused,
}

impl actix::Message for PlayerMessages {
    type Result = ();
}

pub struct Player {
    gain: f64,
    enable: bool,
    pub proto: actix::Addr<proto::Proto>,
    pipeline: Option<gst::Pipeline>,
}

impl Player {
    pub fn new(proto: actix::Addr<proto::Proto>) -> Self {
        Player {
            gain: 1.0,
            enable: false,
            proto: proto,
            pipeline: None,
        }
    }

    fn stream_stop(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            if pipeline.set_state(gst::State::Null) != gst::StateChangeReturn::Failure {
                info!("Stopping stream");
                self.proto.do_send(PlayerMessages::Flushed);
            }
        }
    }

    fn stream_pause(&mut self) {
        if let Some(ref pipeline) = self.pipeline {
            if pipeline.set_state(gst::State::Paused) != gst::StateChangeReturn::Failure {
                info!("Pausing stream");
                self.proto.do_send(PlayerMessages::Paused);
            }
        }
    }

    fn stream_unpause(&mut self) {
        if let Some(ref pipeline) = self.pipeline {
            if pipeline.set_state(gst::State::Playing) != gst::StateChangeReturn::Failure {
                info!("Resuming stream");
                self.proto.do_send(PlayerMessages::Unpaused);
            }
        }
    }
}

impl actix::Actor for Player {
    type Context = actix::Context<Self>;

    fn started(&mut self, _ctx: &mut actix::Context<Self>) {
        match gst::init() {
            Ok(_) => (),
            Err(e) => error!("Unable to initialise GStreamer: {}", e),
        }
    }
}

impl actix::Handler<PlayerControl> for Player {
    type Result = ();

    fn handle(&mut self, msg: PlayerControl, ctx: &mut actix::Context<Self>) {
        const IBUF_SIZE: u32 = 2 * 1024 * 1024; // bytes
        const OBUF_SIZE: u64 = 10_000_000_000; // nanoseconds

        match msg {
            PlayerControl::Gain(gain_left, gain_right) => {
                self.gain = if gain_left > gain_right {
                    gain_left
                } else {
                    gain_right
                };
                info!("Setting gain to {}", self.gain);
            }

            PlayerControl::Enable(enable) => {
                info!("Setting enable to {}", enable);
                self.enable = enable;
            }

            PlayerControl::Stream {
                autostart,
                threshold,
                output_threshold,
                replay_gain,
                server_port,
                server_ip,
                control_ip,
                http_headers,
            } => {
                info!("Got stream request");

                let mut elements = HashMap::new();
                elements.extend(vec![
                    ("source", gst::ElementFactory::make("souphttpsrc", "source")),
                    ("counter", gst::ElementFactory::make("identity", "counter")),
                    ("ibuf", gst::ElementFactory::make("queue", "ibuf")),
                    ("decoder", gst::ElementFactory::make("decodebin", "decoder")),
                    (
                        "converter",
                        gst::ElementFactory::make("audioconvert", "converter"),
                    ),
                    (
                        "resampler",
                        gst::ElementFactory::make("audioresample", "resampler"),
                    ),
                    ("volume", gst::ElementFactory::make("volume", "volume")),
                    ("obuf", gst::ElementFactory::make("queue", "obuf")),
                    ("sink", gst::ElementFactory::make("autoaudiosink", "sink")),
                ]);

                if elements.values().any(|e| e.is_none()) {
                    error!("Unable to instnciate stream elements");
                    return;
                }

                // From this point on element unwraps are safe

                let elements: HashMap<&str, gst::Element> =
                    elements.into_iter().map(|(k, v)| (k, v.unwrap())).collect();

                let pipeline = gst::Pipeline::new("Storm Player");
                for element in elements.values() {
                    if let Err(_) = pipeline.add(element) {
                        error!("Error adding elements to pipeline");
                        return;
                    }
                }

                for elems in ["source", "counter", "ibuf", "decoder"].windows(2) {
                    if let Err(_) = elements
                        .get(elems[0])
                        .unwrap()
                        .link(elements.get(elems[1]).unwrap())
                    {
                        error!("Cannot link elements");
                        return;
                    }
                }

                for elems in ["converter", "resampler", "volume", "obuf", "sink"].windows(2) {
                    if let Err(_) = elements
                        .get(elems[0])
                        .unwrap()
                        .link(elements.get(elems[1]).unwrap())
                    {
                        error!("Cannot link elements");
                        return;
                    }
                }

                let server_ip = if server_ip == Ipv4Addr::new(0, 0, 0, 0) {
                    control_ip
                } else {
                    server_ip
                };

                let get = http_headers
                    .lines()
                    .nth(0)
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();

                info!("http://{}:{}{}", server_ip, server_port, get);

                let source = elements.get("source").unwrap();
                source
                    .set_property(
                        "location",
                        &format!("http://{}:{}{}", server_ip, server_port, get),
                    )
                    .unwrap();
                source
                    .set_property("user-agent", &"Storm".to_owned())
                    .unwrap();

                let ibuf = elements.get("ibuf").unwrap();
                ibuf.set_property("max-size-bytes", &IBUF_SIZE).unwrap();
                ibuf.set_property("min-threshold-bytes", &threshold)
                    .unwrap();

                let obuf = elements.get("obuf").unwrap();
                obuf.set_property("max-size-time", &OBUF_SIZE).unwrap();
                obuf.set_property("min-threshold-time", &output_threshold)
                    .unwrap();

                info!("Threshold: {}\nOutput threshold: {}", threshold, output_threshold);

                let volume = elements.get("volume").unwrap();
                let gain = if replay_gain < 0.0001 {
                    self.gain
                } else {
                    replay_gain
                };
                volume.set_property("volume", &gain).unwrap();
                volume.set_property("mute", &!self.enable).unwrap();

                let decoder = elements.get("decoder").unwrap();
                let converter_weak = elements.get("converter").unwrap().downgrade();
                decoder.connect_pad_added(move |_, src_pad| {
                    let converter = match converter_weak.upgrade() {
                        Some(converter) => converter,
                        None => return,
                    };

                    let sink_pad = converter
                        .get_static_pad("sink")
                        .expect("Failed to get static sink pad from convert");
                    if sink_pad.is_linked() {
                        info!("We are already linked. Ignoring.");
                        return;
                    }

                    let new_pad_caps = src_pad
                        .get_current_caps()
                        .expect("Failed to get caps of new pad.");
                    let new_pad_struct = new_pad_caps
                        .get_structure(0)
                        .expect("Failed to get first structure of caps.");
                    let new_pad_type = new_pad_struct.get_name();

                    if new_pad_type.starts_with("audio/x-raw") {
                        let _ = src_pad.link(&sink_pad);
                    }
                });

                if autostart {
                    if pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
                        error!("Unable to set the pipeline to the Playing state");
                    }
                }
                self.pipeline = Some(pipeline);
            }

            PlayerControl::Stop => {
                self.stream_stop();
            }

            // TODO: pause for fixed period
            PlayerControl::Pause => {
                self.stream_pause();
            }

            // TODO: unpause at time
            PlayerControl::Unpause => {
                self.stream_unpause();
            }
        }
    }
}
