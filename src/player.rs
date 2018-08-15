use actix;
// use glib;
// use glib::translate::ToGlib;
use gst;
use gst::prelude::*;
use gst::MessageView;

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
    Pause(bool),
    Unpause(bool),
}

impl actix::Message for PlayerControl {
    type Result = ();
}

pub enum PlayerMessages {
    Flushed,
    Paused,
    Unpaused,
    Eos,
    Established,
    Headers(u8),
    Error,
    Start,
    Streamdata {
        position: u64,
        fullness: u32,
        output_buffer_fullness: u32,
    },
    Bufsize(usize),
    // Underrun,
    // Outputunderrun,
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
                if let Some(pipeline) = self.pipeline.clone() {
                    if let Some(volume) = pipeline.get_by_name("volume") {
                        volume.set_property("volume", &self.gain).unwrap();
                    }
                }
            }

            PlayerControl::Enable(enable) => {
                info!("Setting enable to {}", enable);
                self.enable = enable;
                if let Some(pipeline) = self.pipeline.clone() {
                    if let Some(volume) = pipeline.get_by_name("volume") {
                        volume.set_property("mute", &!enable).unwrap();
                    }
                }
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
                info!("Got stream request, autostart: {}", autostart);

                self.stream_stop();

                let mut elements = HashMap::new();
                elements.extend(vec![
                    ("source", gst::ElementFactory::make("souphttpsrc", "source")),
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

                let pipeline = gst::Pipeline::new("stormpipe");
                for element in elements.values() {
                    if let Err(_) = pipeline.add(element) {
                        error!("Error adding elements to pipeline");
                        return;
                    }
                }

                for elems in ["source", "ibuf", "decoder"].windows(2) {
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

                let get = if let Some(line) = http_headers.lines().nth(0) {
                    if let Some(get) = line.split_whitespace().nth(1) {
                        get.to_owned()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                info!("http://{}:{}{}", server_ip, server_port, get);

                if let Some(source) = elements.get("source") {
                    source
                        .set_property(
                            "location",
                            &format!("http://{}:{}{}", server_ip, server_port, get),
                        )
                        .unwrap();
                    source
                        .set_property("user-agent", &"Storm".to_owned())
                        .unwrap();

                    if let Some(src_pad) = source.get_static_pad("src") {
                        let proto = self.proto.clone();
                        src_pad.add_probe(
                            gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
                            move |_, probe_info| {
                                let buf_size = match probe_info.data {
                                    Some(gst::PadProbeData::Buffer(ref buffer)) => {
                                        buffer.get_size()
                                    }
                                    Some(gst::PadProbeData::BufferList(ref buflist)) => {
                                        buflist.iter().map(|b| b.get_size()).sum()
                                    }
                                    _ => 0,
                                };
                                proto.do_send(PlayerMessages::Bufsize(buf_size));
                                gst::PadProbeReturn::Ok
                            },
                        );
                    }
                }

                // let underrun_id = if let Some(ibuf) = elements.get("ibuf") {
                //     ibuf.set_property("max-size-bytes", &threshold).unwrap();
                //     let proto = self.proto.clone();
                //     if let Ok(underrun_id) = ibuf.connect("underrun", true, move |_| {
                //         proto.do_send(PlayerMessages::Underrun);
                //         None
                //     }) {
                //         underrun_id.to_glib()
                //     } else {
                //         0
                //     }
                // } else {
                //     0
                // };

                if let Some(ibuf) = elements.get("ibuf") {
                    ibuf.set_property("max-size-bytes", &(&threshold * 32))
                        .unwrap();
                };

                if let Some(obuf) = elements.get("obuf") {
                    obuf.set_property("max-size-time", &(&output_threshold * 1))
                        .unwrap();
                };

                if let Some(volume) = elements.get("volume") {
                    let gain = if replay_gain < 0.0001 {
                        self.gain
                    } else {
                        replay_gain
                    };
                    volume.set_property("volume", &gain).unwrap();
                    volume.set_property("mute", &!self.enable).unwrap();
                }

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

                let proto = self.proto.clone();
                let bus = pipeline.get_bus().unwrap();
                let pipeline_weak = pipeline.downgrade();
                ::std::thread::spawn(move || loop {
                    let pipeline = match pipeline_weak.upgrade() {
                        Some(pipeline) => pipeline,
                        None => return,
                    };

                    let msg = bus.timed_pop(gst::ClockTime::from_mseconds(100));
                    // println!("{:?}", msg);
                    match msg {
                        Some(msg) => match msg.view() {
                            MessageView::Error(error) => {
                                error!(
                                    "Stream error: {}",
                                    if let Some(e) = error.get_debug() {
                                        e
                                    } else {
                                        "undefined".to_owned()
                                    }
                                );
                                proto.do_send(PlayerMessages::Error);
                                break;
                            }

                            MessageView::Eos(..) => {
                                // if let Some(ibuf) = pipeline.get_by_name("ibuf") {
                                //     info!("Disconnecting: {}", underrun_id);
                                //     ibuf.disconnect(glib::translate::from_glib(underrun_id));
                                // }
                                info!("End of stream stected");
                                proto.do_send(PlayerMessages::Eos);
                            }

                            MessageView::Element(element) => {
                                if let Some(source) = element.get_src() {
                                    if source.get_name() == "source" {
                                        if let Some(structure) = element.get_structure() {
                                            if structure.get_name() == "http-headers" {
                                                proto.do_send(PlayerMessages::Established);
                                                let crlf = structure.iter().count() as u8;
                                                proto.do_send(PlayerMessages::Headers(crlf));
                                            }
                                        }
                                    }
                                }
                            }

                            MessageView::StateChanged(state) => {
                                if let Some(source) = state.get_src() {
                                    if source.get_name() == "stormpipe" {
                                        if state.get_current() == gst::State::Null {
                                            info!("Pipeline moved to null state");
                                            break;
                                        }
                                    }
                                }
                            }

                            MessageView::StreamStart(..) => {
                                proto.do_send(PlayerMessages::Start);
                            }

                            _ => (),
                        },

                        None => {
                            proto.do_send(PlayerMessages::Streamdata {
                                position: query_pos(&pipeline),
                                fullness: buffer_fullness(&pipeline, "ibuf"),
                                output_buffer_fullness: buffer_fullness(&pipeline, "obuf"),
                            });
                        }
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

            PlayerControl::Pause(quiet) => {
                if let Some(ref pipeline) = self.pipeline {
                    info!("Pausing stream");
                    if pipeline.set_state(gst::State::Paused) != gst::StateChangeReturn::Failure {
                        if !quiet {
                            self.proto.do_send(PlayerMessages::Paused);
                        }
                    }
                }
            }

            PlayerControl::Unpause(quiet) => {
                if let Some(ref pipeline) = self.pipeline {
                    info!("Resuming stream");
                    if pipeline.set_state(gst::State::Playing) != gst::StateChangeReturn::Failure {
                        if !quiet {
                            self.proto.do_send(PlayerMessages::Unpaused);
                        }
                    }
                }
            }
        }
    }
}

fn query_pos(pipeline: &gst::Pipeline) -> u64 {
    let mut q = gst::Query::new_position(gst::Format::Time);
    if pipeline.query(&mut q) {
        match q.get_result().try_into_time() {
            Ok(pos) => pos.mseconds().unwrap_or(0),
            _ => 0,
        }
    } else {
        0
    }
}

fn buffer_fullness(pipeline: &gst::Pipeline, buffer: &str) -> u32 {
    match pipeline.get_by_name(buffer) {
        Some(buf) => match buf.get_property("current-level-bytes") {
            Ok(bytes) => bytes.get().unwrap_or(0),
            _ => 0,
        },
        _ => 0,
    }
}
