use actix;
use gst;
use gst::prelude::*;
// use gst::{BinExtManual, ElementExt};

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
        http_headers: String,
    },
}

impl actix::Message for PlayerControl {
    type Result = ();
}

pub struct Player {
    gain: f64,
    enable: bool,
    pipeline: Option<gst::Pipeline>,
}

impl Player {
    pub fn new() -> Self {
        Player {
            gain: 1.0,
            enable: false,
            pipeline: None,
        }
    }

    fn build_pipeline(&self, location: String) -> gst::Pipeline {
        let source = gst::ElementFactory::make("souphttpsrc", "source").unwrap();
        let counter = gst::ElementFactory::make("identity", "counter").unwrap();
        let ibuf = gst::ElementFactory::make("queue", "ibuf").unwrap();
        let decoder = gst::ElementFactory::make("decodebin", "decoder").unwrap();
        let converter = gst::ElementFactory::make("audioconvert", "converter").unwrap();
        let resampler = gst::ElementFactory::make("audioresample", "resampler").unwrap();
        let obuf = gst::ElementFactory::make("queue", "obuf").unwrap();
        let sink = gst::ElementFactory::make("autoaudiosink", "sink").unwrap();

        let pipeline = gst::Pipeline::new("Storm Player");
        pipeline
            .add_many(&[
                &source, &counter, &ibuf, &decoder, &converter, &resampler, &obuf, &sink,
            ])
            .unwrap();

        pipeline
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
                http_headers,
            } => {
                let source = gst::ElementFactory::make("souphttpsrc", "source").unwrap();
                let counter = gst::ElementFactory::make("identity", "counter").unwrap();
                let ibuf = gst::ElementFactory::make("queue", "ibuf").unwrap();
                let decoder = gst::ElementFactory::make("decodebin", "decoder").unwrap();
                let volume = gst::ElementFactory::make("volume", "volume").unwrap();
                let converter = gst::ElementFactory::make("audioconvert", "converter").unwrap();
                let resampler = gst::ElementFactory::make("audioresample", "resampler").unwrap();
                let obuf = gst::ElementFactory::make("queue", "obuf").unwrap();
                let sink = gst::ElementFactory::make("autoaudiosink", "sink").unwrap();

                let pipeline = gst::Pipeline::new("Storm Player");
                pipeline
                    .add_many(&[
                        &source, &counter, &ibuf, &decoder, &volume, &converter, &resampler, &obuf, &sink,
                    ])
                    .unwrap();

                source.link(&counter).unwrap();
                counter.link(&ibuf).unwrap();
                ibuf.link(&decoder).unwrap();
                // decoder.link(&volume).unwrap();
                volume.link(&converter).unwrap();
                converter.link(&resampler).unwrap();
                resampler.link(&obuf).unwrap();
                obuf.link(&sink).unwrap();

                let get_line = http_headers.lines().nth(0).unwrap();
                let get = get_line.split_whitespace().nth(1).unwrap();

                source.set_property("location", &format!("http://{}:{}{}", server_ip, server_port, get)).unwrap();
                // source.set_property("extra-headers", &x_head).unwrap();
                // source.set_property("user-agent", &"Storm".to_owned()).unwrap();
                volume.set_property("volume", &self.gain).unwrap();
                volume.set_property("mute", &!self.enable).unwrap();
                let _ = pipeline.set_state(gst::State::Playing);
                self.pipeline = Some(pipeline);
            }
        }
    }
}
