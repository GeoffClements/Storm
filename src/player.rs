use actix;
use gst;
use gst::prelude::*;
use gst::MessageView;
use thread_control;

use proto;

use std::collections::HashMap;
use std::net::Ipv4Addr;

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
    Overrun,
    // Underrun,
    // Outputunderrun,
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
        }
    }

    fn delete_source(&mut self) {
        if let Some(source) = self.pipeline.get_by_name("source") {
            if let Some(ibuf) = self.pipeline.get_by_name("ibuf") {
                source.unlink(&ibuf);
                if !source.set_state(gst::State::Null).is_err() {
                    info!("Destroying: {}", source.get_name());
                    let _ = self.pipeline.remove(&source);
                }
            }
        }
    }
}

impl actix::Actor for Player {
    type Context = actix::Context<Self>;

    fn started(&mut self, _ctx: &mut actix::Context<Self>) {
        if self.pipeline.get_by_name("sink").is_some() {
            return;
        }

        // Audio Sink
        let sink = {
            match self.output_device.service {
                AudioService::Auto => gst::ElementFactory::make("autoaudiosink", Some("sink")),
                AudioService::Alsa => gst::ElementFactory::make("alsasink", Some("sink")),
                AudioService::Pulse => gst::ElementFactory::make("pulsesink", Some("sink")),
            }
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
        if self.pipeline.add(&concat).is_err() {
            return;
        };

        if concat.link(&obuf).is_err() {
            return;
        };

        //Set up periodic message
        let proto = self.proto.clone();
        let (flag, control) = thread_control::make_pair();
        self.thread = Some(control);
        let bus = self.pipeline.get_bus().unwrap();
        let pipeline_weak = self.pipeline.downgrade();
        let streams = self.streams.clone(); //Won't work
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
                        }

                        // MessageView::Eos(..) => {
                        //     info!("End of stream detected");
                        //     proto.do_send(PlayerMessages::Eos);
                        // }
                        // MessageView::Element(element) => {
                        //     if let Some(source) = element.get_src() {
                        //         if source.get_name() == "source" {
                        //             if let Some(structure) = element.get_structure() {
                        //                 if structure.get_name() == "http-headers" {
                        //                     proto.do_send(PlayerMessages::Established);
                        //                     let crlf = structure.iter().count() as u8;
                        //                     proto.do_send(PlayerMessages::Headers(crlf));
                        //                 }
                        //             }
                        //         }
                        //     }
                        // }

                        // MessageView::StateChanged(state) => {
                        //     if let Some(source) = state.get_src() {
                        //         if source.get_name() == "stormpipe" {
                        //             if state.get_current() == gst::State::Null {
                        //                 info!("Pipeline moved to null state");
                        //             }
                        //         }
                        //     }
                        // }

                        // MessageView::StreamStart(..) => {
                        //     info!("Stream is starting");
                        //     proto.do_send(PlayerMessages::Start);
                        // }
                        MessageView::Latency(..) => {
                            info!("Recalculating latency");
                            let _ = pipeline.recalculate_latency();
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
            }
        });

        //         let decoder = elements.get("decoder").unwrap();
        //         let converter_weak = elements.get("converter").unwrap().downgrade();
        //         decoder.connect_pad_added(move |_, src_pad| {
        //             let converter = match converter_weak.upgrade() {
        //                 Some(converter) => converter,
        //                 None => return,
        //             };

        //             let sink_pad = converter
        //                 .get_static_pad("sink")
        //                 .expect("Failed to get static sink pad from convert");
        //             if sink_pad.is_linked() {
        //                 info!("We are already linked. Ignoring.");
        //                 return;
        //             }

        //             let new_pad_caps = src_pad
        //                 .get_current_caps()
        //                 .expect("Failed to get caps of new pad.");
        //             let new_pad_struct = new_pad_caps
        //                 .get_structure(0)
        //                 .expect("Failed to get first structure of caps.");
        //             let new_pad_type = new_pad_struct.get_name();

        //             if new_pad_type.starts_with("audio/x-raw") {
        //                 info!("Linking decoder to converter");
        //                 let _ = src_pad.link(&sink_pad);
        //             }
        //         });
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

                // if self.pipeline.set_state(gst::State::Ready) == gst::StateChangeReturn::Failure {
                //     error!("Unable to set the pipeline to the Playing state");
                // }
                //self.fill_pipeline();

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
                // let binsrcpad = gst::GhostPad::new_no_target(None, gst::PadDirection::Src);

                let decoder = gst::ElementFactory::make("decodebin", Some("decoder")).unwrap();
                if stream.add(&decoder).is_err() {
                    return;
                };

                let concat_weak = self.pipeline.get_by_name("concat").unwrap().downgrade();
                let stream_weak = stream.downgrade();
                decoder.connect_pad_added(move |_, src_pad| {
                    let concat = match concat_weak.upgrade() {
                        Some(concat) => concat,
                        None => return,
                    };

                    let stream = stream_weak.upgrade().unwrap();

                    if let Some(sink_pad) = concat.get_compatible_pad(src_pad, None) {
                        let g_pad = gst::GhostPad::new(None, src_pad).unwrap();
                        let _ = g_pad.set_active(true);
                        if stream.add_pad(&g_pad).is_ok() {
                            let _ = g_pad.link(&sink_pad);
                        }
                    }
                });


                // TODO: Override buffer size with command line option
                let ibuf = gst::ElementFactory::make("queue", Some("ibuf")).unwrap();
                ibuf.set_property("max-size-bytes", &(&threshold)).unwrap();
                if stream.add(&ibuf).is_ok() {
                    if ibuf.link(&decoder).is_err() {
                        return;
                    }
                } else {
                    return;
                };

                let source = gst::ElementFactory::make("souphttpsrc", None).unwrap();
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
                    src_pad.add_probe(gst::PadProbeType::EVENT_BOTH, move |_, probe_info| {
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

                if self.streams.len() > 1 {
                    if let Some(old_stream) = self.streams.pop() {
                        if old_stream.set_state(gst::State::Null).is_ok() {
                            let _ = self.pipeline.remove(&old_stream);
                            if let Some(pad) =
                                self.pipeline.find_unlinked_pad(gst::PadDirection::Sink)
                            {
                                if pad.get_parent().unwrap().get_name().as_str() == "concat" {
                                    let concat = self.pipeline.get_by_name("concat").unwrap();
                                    concat.release_request_pad(&pad)
                                }
                            }
                        }
                    }
                };

                self.streams.insert(0, stream);

                // if self.pipeline.get_by_name("source").is_none() {
                //     let _ = self.pipeline.set_state(gst::State::Ready);
                //     match gst::ElementFactory::make("souphttpsrc", Some("source")) {
                //         Some(source) => {
                //             info!("Creating new source");
                //             source
                //                 .set_property("user-agent", &"Storm".to_owned())
                //                 .unwrap();
                //             source.set_property("location", &location).unwrap();
                //             self.pipeline.add(&source).unwrap();
                //             if let Some(ibuf) = self.pipeline.get_by_name("ibuf") {
                //                 source.link(&ibuf).unwrap();
                //             }
                //             let _ = source.sync_state_with_parent();
                //             if let Some(src_pad) = source.get_static_pad("src") {
                //                 let proto = self.proto.clone();
                //                 src_pad.add_probe(
                //                     gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
                //                     move |_, probe_info| {
                //                         let buf_size = match probe_info.data {
                //                             Some(gst::PadProbeData::Buffer(ref buffer)) => {
                //                                 buffer.get_size()
                //                             }
                //                             Some(gst::PadProbeData::BufferList(ref buflist)) => {
                //                                 buflist.iter().map(|b| b.get_size()).sum()
                //                             }
                //                             _ => 0,
                //                         };
                //                         proto.do_send(PlayerMessages::Bufsize(buf_size));
                //                         gst::PadProbeReturn::Ok
                //                     },
                //                 );
                //                 let proto = self.proto.clone();
                //                 src_pad.add_probe(
                //                     gst::PadProbeType::EVENT_BOTH,
                //                     move |_, probe_info| {
                //                         if let Some(ref probe_data) = probe_info.data {
                //                             if let gst::PadProbeData::Event(event) = probe_data {
                //                                 if event.get_type() == gst::EventType::Eos {
                //                                     proto.do_send(PlayerMessages::Eos);
                //                                 }
                //                             }
                //                         }
                //                         gst::PadProbeReturn::Ok
                //                     },
                //                 );
                //             };
                //             let _ = source.sync_state_with_parent();
                //         }
                //         None => {
                //             error!("Unable to create source element");
                //             ::std::process::exit(1);
                //         }
                //     }
                // } else {
                //     // let source = self.pipeline.get_by_name("source").unwrap();
                //     // let _ = source.set_state(gst::State::Ready);
                //     // source.set_property("location", &location).unwrap();
                //     // let _ = source.sync_state_with_parent();
                // }

                // if !self.pipeline.set_state(gst::State::Playing).is_err() {
                //     error!("Unable to set the pipeline to the Playing state");
                // }

                if autostart {
                    if self.pipeline.set_state(gst::State::Playing).is_ok() {
                        self.proto.do_send(PlayerMessages::Start);
                    }
                }
            }

            PlayerControl::Stop => {
                if !self.pipeline.set_state(gst::State::Ready).is_err() {
                    info!("Stopped stream");
                    self.proto.do_send(PlayerMessages::Flushed);
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
            } // PlayerControl::Deletesource => {
              //     if self.sources.len() > 1 {
              //         self.delete_last_source()
              //     }
              // }
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

fn buffer_fullness(pipeline: &gst::Pipeline, buffer: &str) -> u32 {
    match pipeline.get_by_name(buffer) {
        Some(buf) => match buf.get_property("current-level-bytes") {
            Ok(bytes) => bytes.get().unwrap_or(0),
            _ => 0,
        },
        _ => 0,
    }
}
