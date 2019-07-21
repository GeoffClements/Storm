use actix;
use actix::{Actor, AsyncContext};
use gst;
use gst::prelude::*;
use gst::MessageView;
use thread_control;

use proto;

use std::net::Ipv4Addr;
use std::time::Duration;

#[derive(Copy, Clone)]
enum AudioService {
    Auto,
    Alsa,
    Pulse,
}

#[derive(Clone)]
pub struct AudioDevice {
    service: AudioService,
    device: Option<String>,
}

impl Default for AudioDevice {
    fn default() -> Self {
        AudioDevice {
            service: AudioService::Auto,
            device: None,
        }
    }
}

impl<'a> From<Vec<&'a str>> for AudioDevice {
    fn from(v: Vec<&str>) -> Self {
        // info!("Output-device: {:?}", v);
        if v.len() == 0 {
            return AudioDevice::default();
        }

        match v[0].to_lowercase().as_str() {
            "auto" => AudioDevice::default(),
            "alsa" => {
                let device = if v.len() > 1 {
                    Some(v[1].to_owned())
                } else {
                    None
                };
                AudioDevice {
                    service: AudioService::Alsa,
                    device: device,
                }
            }
            "pulse" => {
                let device = if v.len() > 1 {
                    Some(v[1].to_owned())
                } else {
                    None
                };
                AudioDevice {
                    service: AudioService::Pulse,
                    device: device,
                }
            }
            _ => {
                warn!("Unable to parse output device, falling back to auto output selection");
                AudioDevice::default()
            }
        }
    }
}

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
    Skip(u32),
    Cleanup(bool),
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
    Sendstatus,
}

impl actix::Message for PlayerMessages {
    type Result = ();
}

pub struct Player {
    gain: f64,
    enable: bool,
    output_device: AudioDevice,
    thread: Option<thread_control::Control>,
    pub proto: actix::Addr<proto::Proto>,
    pipeline: gst::Pipeline,
    streams: Vec<gst::Bin>,
    crypt: actix::Addr<Crypt>,
}

impl Player {
    pub fn new(proto: actix::Addr<proto::Proto>, output_device: AudioDevice) -> Self {
        if let Err(e) = gst::init() {
            error!("Unable to initialise GStreamer: {}", e);
            ::std::process::exit(1);
        }

        Player {
            gain: 1.0,
            enable: false,
            output_device: output_device,
            thread: None,
            proto: proto,
            pipeline: gst::Pipeline::new(Some("stormpipe")),
            streams: Vec::with_capacity(2),
            crypt: Crypt::new().start(),
        }
    }

    fn clean_streams(&mut self, all: bool) {
        if all {
            let _ = self.pipeline.set_state(gst::State::Ready);
        };

        while self.streams.len() > {
            if all {
                0
            } else {
                1
            }
        } {
            if let Some(old_stream) = self.streams.pop() {
                let source = old_stream.get_by_name("source").unwrap();
                source.set_state(gst::State::Null).unwrap();
                if old_stream.set_state(gst::State::Null).is_ok() {
                    let _ = self.pipeline.remove(&old_stream);
                    while let Some(pad) = self.pipeline.find_unlinked_pad(gst::PadDirection::Sink) {
                        if let Some(elem) = pad.get_parent_element() {
                            if elem.get_name() == "concat" {
                                elem.release_request_pad(&pad)
                            }
                        }
                    }
                    self.crypt.do_send(Corpse { corpse: old_stream });
                }
            }
        }
    }
}

impl actix::Actor for Player {
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut actix::Context<Self>) {
        if self.pipeline.get_by_name("sink").is_some() {
            return;
        }

        // Audio Sink
        let sink = match self.output_device.service {
            AudioService::Auto => gst::ElementFactory::make("autoaudiosink", Some("sink")),
            AudioService::Alsa => gst::ElementFactory::make("alsasink", Some("sink")),
            AudioService::Pulse => gst::ElementFactory::make("pulsesink", Some("sink")),
        }
        .unwrap();

        if let Some(ref device) = self.output_device.device {
            sink.set_property("device", &device).unwrap();
        };

        {
            let service = match self.output_device.service {
                AudioService::Alsa => "ALSA",
                AudioService::Pulse => "PULSEAUDIO",
                _ => "AUTO",
            };
            let device = if let Ok(prop) = sink.get_property("device-name") {
                prop.get().unwrap_or("default".to_owned())
            } else {
                "default".to_owned()
            };

            info!("Using audio service: {} with device: {}", service, device);
        }

        if self.pipeline.add(&sink).is_err() {
            return;
        };

        // Audio Resample
        let resampler = gst::ElementFactory::make("audioresample", Some("resampler")).unwrap();
        if self.pipeline.add(&resampler).is_err() {
            return;
        };

        if resampler.link(&sink).is_err() {
            return;
        };

        // Audio Converter
        let converter = gst::ElementFactory::make("audioconvert", Some("converter")).unwrap();
        if self.pipeline.add(&converter).is_err() {
            return;
        };

        if converter.link(&resampler).is_err() {
            return;
        };

        // Volume
        let volume = gst::ElementFactory::make("volume", Some("volume")).unwrap();
        if self.pipeline.add(&volume).is_err() {
            return;
        };

        if volume.link(&converter).is_err() {
            return;
        };

        // Output Buffer
        let obuf = gst::ElementFactory::make("queue", Some("obuf")).unwrap();
        if self.pipeline.add(&obuf).is_err() {
            return;
        };

        if obuf.link(&volume).is_err() {
            return;
        };

        // Concat
        let concat = gst::ElementFactory::make("concat", Some("concat")).unwrap();
        let concat_src = concat.get_static_pad("src").unwrap();
        let proto = self.proto.clone();
        let player = ctx.address().clone();
        concat_src.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, probe_info| {
            if let Some(ref probe_data) = probe_info.data {
                if let gst::PadProbeData::Event(event) = probe_data {
                    if event.get_type() == gst::EventType::StreamStart {
                        proto.do_send(PlayerMessages::Start);
                        proto.do_send(PlayerMessages::Sendstatus);
                        player.do_send(PlayerControl::Cleanup(false));
                    }
                }
            }
            gst::PadProbeReturn::Ok
        });

        if self.pipeline.add(&concat).is_err() {
            return;
        };

        if concat.link(&obuf).is_err() {
            return;
        };

        //Set up periodic message
        let proto = self.proto.clone();
        let player = ctx.address().clone();
        let (flag, control) = thread_control::make_pair();
        self.thread = Some(control);
        let bus = self.pipeline.get_bus().unwrap();
        let pipeline_weak = self.pipeline.downgrade();
        ::std::thread::spawn(move || {
            let pipeline = match pipeline_weak.upgrade() {
                Some(pipeline) => pipeline,
                None => return,
            };

            loop {
                let msg = bus.timed_pop(gst::ClockTime::from_mseconds(100));
                if !flag.is_alive() {
                    break;
                }

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
                            player.do_send(PlayerControl::Cleanup(true));
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

                        MessageView::Latency(..) => {
                            info!("Recalculating latency");
                            let _ = pipeline.recalculate_latency();
                        }

                        // MessageView::Tag(tag) => {
                        //     let tags = tag.get_tags();
                        //     info!("{}", tags.to_string());
                        // }
                        _ => (),
                    },

                    None => {
                        let (ibuf_fullness, obuf_fullness) = buffer_fullness(&pipeline);
                        proto.do_send(PlayerMessages::Streamdata {
                            position: query_pos(&pipeline),
                            fullness: ibuf_fullness,
                            output_buffer_fullness: obuf_fullness,
                        });
                    }
                }
            }
        });
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
                self.gain = if self.gain > 1.0 { 1.0 } else { self.gain };
                info!("Setting gain to {}", self.gain);
                if let Some(volume) = self.pipeline.get_by_name("volume") {
                    volume.set_property("volume", &self.gain).unwrap();
                }
            }

            PlayerControl::Enable(enable) => {
                info!("Setting enable to {}", enable);
                self.enable = enable;
                if let Some(volume) = self.pipeline.get_by_name("volume") {
                    volume.set_property("mute", &!enable).unwrap();
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

                let location = format!("http://{}:{}{}", server_ip, server_port, get);
                info!("{}", location);

                // Create stream decode elements
                let stream = gst::Bin::new(None);

                let decoder = gst::ElementFactory::make("decodebin", Some("decoder")).unwrap();
                if stream.add(&decoder).is_err() {
                    return;
                };

                let concat_weak = self.pipeline.get_by_name("concat").unwrap().downgrade();
                let stream_weak = stream.downgrade();
                decoder.connect_pad_added(move |_, src_pad| {
                    let concat = concat_weak.upgrade().unwrap();
                    let stream = stream_weak.upgrade().unwrap();

                    if let Some(sink_pad) = concat.get_compatible_pad(src_pad, None) {
                        let g_pad = gst::GhostPad::new(None, src_pad).unwrap();
                        let _ = g_pad.set_active(true);
                        if stream.add_pad(&g_pad).is_ok() {
                            let _ = g_pad.link(&sink_pad);
                        }
                    }
                });

                let ibuf = gst::ElementFactory::make("queue", Some("ibuf")).unwrap();
                ibuf.set_property("max-size-bytes", &(&threshold)).unwrap();
                if stream.add(&ibuf).is_ok() {
                    if ibuf.link(&decoder).is_err() {
                        return;
                    }
                } else {
                    return;
                };

                let source = gst::ElementFactory::make("souphttpsrc", Some("source")).unwrap();
                if stream.add(&source).is_ok() {
                    if source.link(&ibuf).is_err() {
                        return;
                    }
                } else {
                    return;
                }
                source
                    .set_property("user-agent", &"Storm".to_owned())
                    .unwrap();
                source.set_property("location", &location).unwrap();

                if let Some(obuf) = self.pipeline.get_by_name("obuf") {
                    obuf.set_property("max-size-time", &(&output_threshold))
                        .unwrap();
                };

                if let Some(volume) = self.pipeline.get_by_name("volume") {
                    let gain = if replay_gain < 0.0001 {
                        self.gain
                    } else {
                        replay_gain
                    };
                    volume.set_property("volume", &gain).unwrap();
                    volume.set_property("mute", &!self.enable).unwrap();
                }

                if let Some(src_pad) = source.get_static_pad("src") {
                    let proto = self.proto.clone();
                    src_pad.add_probe(
                        gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
                        move |_, probe_info| {
                            let buf_size = match probe_info.data {
                                Some(gst::PadProbeData::Buffer(ref buffer)) => buffer.get_size(),
                                Some(gst::PadProbeData::BufferList(ref buflist)) => {
                                    buflist.iter().map(|b| b.get_size()).sum()
                                }
                                _ => 0,
                            };
                            proto.do_send(PlayerMessages::Bufsize(buf_size));
                            gst::PadProbeReturn::Ok
                        },
                    );

                    let proto = self.proto.clone();
                    src_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, probe_info| {
                        if let Some(ref probe_data) = probe_info.data {
                            if let gst::PadProbeData::Event(event) = probe_data {
                                if event.get_type() == gst::EventType::Eos {
                                    proto.do_send(PlayerMessages::Eos);
                                }
                            }
                        }
                        gst::PadProbeReturn::Ok
                    });
                }

                let _ = self.pipeline.add(&stream);
                let _ = stream.sync_state_with_parent();

                self.streams.insert(0, stream);

                if autostart {
                    if self.pipeline.set_state(gst::State::Playing).is_ok() {
                        info!("Autostarting track");
                    }
                }
            }

            PlayerControl::Stop => {
                if !self.pipeline.set_state(gst::State::Ready).is_err() {
                    info!("Stopped stream");
                    self.proto.do_send(PlayerMessages::Flushed);
                    self.clean_streams(true);
                }
            }

            PlayerControl::Pause(quiet) => {
                info!("Pausing stream");
                if !self.pipeline.set_state(gst::State::Paused).is_err() {
                    if !quiet {
                        self.proto.do_send(PlayerMessages::Paused);
                    }
                }
            }

            PlayerControl::Unpause(quiet) => {
                info!("Resuming stream");
                if !self.pipeline.set_state(gst::State::Playing).is_err() {
                    if !quiet {
                        self.proto.do_send(PlayerMessages::Unpaused);
                    }
                }
            }

            PlayerControl::Skip(interval) => {
                let newpos = query_pos(&self.pipeline) + interval as u64;
                info!("Skipping to postition: {}", newpos);
                let flags = {
                    let (_, state, _) = self.pipeline.get_state(gst::ClockTime::none());
                    if state == gst::State::Playing {
                        gst::SeekFlags::SKIP
                    } else {
                        gst::SeekFlags::SKIP | gst::SeekFlags::FLUSH
                    }
                };
                let seek = gst::Event::new_seek(
                    1.0,
                    flags,
                    gst::SeekType::Set,
                    gst::ClockTime::from_mseconds(newpos),
                    gst::SeekType::None,
                    gst::ClockTime::none(),
                )
                .build();
                self.pipeline.send_event(seek);
            }

            PlayerControl::Cleanup(all) => self.clean_streams(all),
        }
    }
}

fn query_pos(pipeline: &gst::Pipeline) -> u64 {
    let mut q = gst::Query::new_position(gst::Format::Time);
    if pipeline.query(&mut q) {
        match q.get_result() {
            gst::GenericFormattedValue::Time(pos) => pos.mseconds().unwrap_or(0),
            _ => 0,
        }
    } else {
        0
    }
}

fn buffer_fullness(pipeline: &gst::Pipeline) -> (u32, u32) {
    pipeline
        .iterate_recurse()
        .fold((0, 0), |fullnesses, element| {
            match element.get_name().as_str() {
                "ibuf" => {
                    let ibuf_fullnesss = match element.get_property("current-level-bytes") {
                        Ok(bytes) => bytes.get().unwrap_or(0),
                        _ => 0,
                    };
                    Ok((
                        if ibuf_fullnesss > fullnesses.0 {
                            ibuf_fullnesss
                        } else {
                            fullnesses.0
                        },
                        fullnesses.1,
                    ))
                }
                "obuf" => {
                    let obuf_fullness = match element.get_property("current-level-bytes") {
                        Ok(bytes) => bytes.get().unwrap_or(0),
                        _ => 0,
                    };
                    Ok((fullnesses.0, obuf_fullness))
                }
                _ => Ok(fullnesses),
            }
        })
        .unwrap_or((0, 0))
}

struct Crypt {
    tombs: Vec<gst::Bin>,
}

impl Crypt {
    pub fn new() -> Self {
        Crypt { tombs: Vec::new() }
    }
}

impl actix::Actor for Crypt {
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Bring out your dead

        ctx.run_interval(Duration::from_secs(1), |crypt, _| {
            if let Some(corpse) = crypt.tombs.pop() {
                match corpse.iterate_elements().fold(true, |is_null, bin| {
                    match bin.get_state(gst::ClockTime::from_useconds(100)) {
                        (Ok(_), state, _) => Ok(is_null && state == gst::State::Null),
                        (Err(_), _, _) => Err(false),
                    }
                }) {
                    Ok(false) | Err(_) => crypt.tombs.insert(0, corpse),
                    _ => info!("Soylent Green! {}", crypt.tombs.len()), //corpse is dropped
                }
            };
        });
    }
}

struct Corpse {
    corpse: gst::Bin,
}

impl actix::Message for Corpse {
    type Result = ();
}

impl actix::Handler<Corpse> for Crypt {
    type Result = ();
    fn handle(&mut self, msg: Corpse, _ctx: &mut actix::Context<Self>) {
        self.tombs.insert(0, msg.corpse);
    }
}
