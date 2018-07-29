use actix;
use gst;
use gst::prelude::*;

use proto;

use std::net::Ipv4Addr;

pub enum PlayerControl {
    Gain(f64, f64),
    Enable(bool),
    Stream {
        autostart: bool,
        threshold: u8,
        output_threshold: u8,
        replay_gain: f64,
        server_port: u16,
        server_ip: Ipv4Addr,
        control_ip: Ipv4Addr,
        http_headers: String,
    },
}

impl actix::Message for PlayerControl {
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

    fn handle(&mut self, msg: PlayerControl, _ctx: &mut actix::Context<Self>) {
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

                // TODO: Too many unwraps
                let source = gst::ElementFactory::make("souphttpsrc", "source").unwrap();
                let counter = gst::ElementFactory::make("identity", "counter").unwrap();
                let ibuf = gst::ElementFactory::make("queue", "ibuf").unwrap();
                let decoder = gst::ElementFactory::make("decodebin", "decoder").unwrap();
                let converter = gst::ElementFactory::make("audioconvert", "converter").unwrap();
                let resampler = gst::ElementFactory::make("audioresample", "resampler").unwrap();
                let volume = gst::ElementFactory::make("volume", "volume").unwrap();
                let obuf = gst::ElementFactory::make("queue", "obuf").unwrap();
                let sink = gst::ElementFactory::make("autoaudiosink", "sink").unwrap();

                let pipeline = gst::Pipeline::new("Storm Player");
                pipeline
                    .add_many(&[
                        &source, &counter, &ibuf, &decoder, &volume, &converter, &resampler, &obuf,
                        &sink,
                    ])
                    .unwrap();

                source.link(&counter).unwrap();
                counter.link(&ibuf).unwrap();
                ibuf.link(&decoder).unwrap();
                // decoder.link(&converter).unwrap();
                converter.link(&resampler).unwrap();
                resampler.link(&volume).unwrap();
                volume.link(&obuf).unwrap();
                obuf.link(&sink).unwrap();

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

                source
                    .set_property(
                        "location",
                        &format!("http://{}:{}{}", server_ip, server_port, get),
                    )
                    .unwrap();
                // source.set_property("extra-headers", &x_head).unwrap();
                source
                    .set_property("user-agent", &"Storm".to_owned())
                    .unwrap();
                volume.set_property("volume", &self.gain).unwrap();
                volume.set_property("mute", &!self.enable).unwrap();

                let pipeline_weak = pipeline.downgrade();
                let decoder_weak = decoder.downgrade();
                decoder.connect_pad_added(move |_, src_pad| {
                    let pipeline = match pipeline_weak.upgrade() {
                        Some(pipeline) => pipeline,
                        None => return,
                    };

                    let decoder = match decoder_weak.upgrade() {
                        Some(convert) => convert,
                        None => return,
                    };

                    info!(
                        "Received new pad {} from {}",
                        src_pad.get_name(),
                        pipeline.get_name()
                    );

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

                    let is_audio = new_pad_type.starts_with("audio/x-raw");
                    if !is_audio {
                        return;
                    }

                    let _ = src_pad.link(&sink_pad);
                });

                if pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
                    error!("Unable to set the pipeline to the Playing state");
                }
                self.pipeline = Some(pipeline);
            }
        }
    }
}
