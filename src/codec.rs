use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use mac_address;
use tokio_codec;

use std::convert::From;
use std::io;
use std::net::Ipv4Addr;

pub struct SlimCodec;

impl tokio_codec::Encoder for SlimCodec {
    type Item = ClientMessage;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.extend(BytesMut::from(item));
        Ok(())
    }
}

impl tokio_codec::Decoder for SlimCodec {
    type Item = ServerMessage;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<ServerMessage>> {
        if buf.len() <= 2 {
            return Ok(None);
        };

        let size = (buf[..2].into_buf().get_u16_be()) as usize;
        if buf.len() < size + 2 {
            return Ok(None);
        };

        buf.split_to(2);
        let msg = buf.split_to(size);

        match msg.into() {
            ServerMessage::Error => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Server data corrupted",
            )),
            msg @ _ => Ok(Some(msg)),
        }
    }
}

pub enum ClientMessage {
    Helo {
        device_id: u8,
        revision: u8,
        mac: mac_address::MacAddress,
        uuid: [u8; 16],
        wlan_channel_list: u16,
        bytes_received: u64,
        // language: u16,
        capabilities: String,
    },
    Stat {
        event_code: String,
        stat_data: StatData,
    },
    Bye(u8),
}

#[derive(Clone, Copy, Default)]
pub struct StatData {
    pub buffer_size: u32,
    pub fullness: u32,
    pub bytes_received: u64,
    pub sig_strength: u16,
    pub jiffies: u32,
    pub output_buffer_size: u32,
    pub output_buffer_fullness: u32,
    pub elapsed_seconds: u32,
    pub voltage: u16,
    pub elapsed_milliseconds: u32,
    pub timestamp: u32,
    pub error_code: u16,
}

impl StatData {
    pub fn make_stat_message(&self, head: &str) -> ClientMessage {
        ClientMessage::Stat {
            event_code: head.to_owned(),
            stat_data: *self,
        }
    }
}

pub enum AudioFormat {
    Pcm,
    Mp3,
    Flac,
    Wma,
    Ogg,
    Aac,
    Alac,
    Unknown,
}

pub enum ServerMessage {
    Serv {
        ip_address: Ipv4Addr,
        sync_group_id: Option<String>,
    },
    Status(u32),
    Stream {
        autostart: bool,
        format: AudioFormat,
        threshold: u8,
        output_threshold: u8,
        replay_gain: f64,
        server_port: u16,
        server_ip: Ipv4Addr,
        http_headers: String,
    },
    Gain(f64, f64),
    Enable(bool),
    Stop,
    Unrecognised(String),
    Error,
}

impl From<ClientMessage> for BytesMut {
    fn from(src: ClientMessage) -> BytesMut {
        let mut buf = Vec::with_capacity(1024);

        match src {
            ClientMessage::Helo {
                device_id,
                revision,
                mac,
                uuid,
                wlan_channel_list,
                bytes_received,
                capabilities,
            } => {
                buf.put("HELO".as_bytes());
                buf.put_u8(device_id);
                buf.put_u8(revision);
                buf.put(mac.bytes().as_ref());
                buf.put(uuid.as_ref());
                buf.put_u16_be(wlan_channel_list);
                buf.put_u64_be(bytes_received);
                buf.put(capabilities.as_bytes());
            }
            ClientMessage::Bye(val) => {
                buf.put("BYE!".as_bytes());
                buf.put_u8(val);
            }
            ClientMessage::Stat {
                event_code,
                stat_data,
            } => {
                buf.put("STAT".as_bytes());
                buf.put(event_code.as_bytes());
                buf.put_u32_be(stat_data.buffer_size);
                buf.put_u32_be(stat_data.fullness);
                buf.put_u64_be(stat_data.bytes_received);
                buf.put_u16_be(stat_data.sig_strength);
                buf.put_u32_be(stat_data.jiffies);
                buf.put_u32_be(stat_data.output_buffer_size);
                buf.put_u32_be(stat_data.output_buffer_fullness);
                buf.put_u32_be(stat_data.elapsed_seconds);
                buf.put_u16_be(stat_data.voltage);
                buf.put_u32_be(stat_data.elapsed_milliseconds);
                buf.put_u32_be(stat_data.timestamp);
                buf.put_u16_be(stat_data.error_code);
            }
        }

        let mut msg_length = Vec::new();
        msg_length.put_u32_le(buf[4..].len() as u32);
        msg_length.into_iter().for_each(|v| buf.insert(4, v));
        buf.into()
    }
}

impl From<BytesMut> for ServerMessage {
    fn from(mut src: BytesMut) -> ServerMessage {
        const GAIN_FACTOR: f64 = 65536.0;
        let msg: String = src.split_to(4).into_iter().map(|c| c as char).collect();

        match msg.as_str() {
            "serv" => {
                if src.len() < 4 {
                    ServerMessage::Error
                } else {
                    let ip_addr = Ipv4Addr::from(src.split_to(4).into_buf().get_u32_be());
                    let sync_group = if src.len() > 0 {
                        Some(
                            src.take()
                                .into_iter()
                                .map(|c| c as char)
                                .collect::<String>(),
                        )
                    } else {
                        None
                    };
                    ServerMessage::Serv {
                        ip_address: ip_addr,
                        sync_group_id: sync_group,
                    }
                }
            }
            "strm" => {
                if src.len() < 24 {
                    return ServerMessage::Error;
                }

                match src[0] as char {
                    't' => {
                        let timestamp = src[14..18].into_buf().get_u32_be();
                        ServerMessage::Status(timestamp)
                    }

                    's' => {
                        let format = match src[2] as char {
                            'p' => AudioFormat::Pcm,
                            'm' => AudioFormat::Mp3,
                            'f' => AudioFormat::Flac,
                            'w' => AudioFormat::Wma,
                            'o' => AudioFormat::Ogg,
                            'a' => AudioFormat::Aac,
                            'l' => AudioFormat::Alac,
                            _ => AudioFormat::Unknown,
                        };
                        let replay_gain = src[14..18].into_buf().get_u32_be() as f64 / GAIN_FACTOR;
                        let http_headers = if src.len() >= 24 {
                            src[24..].into_iter().map(|c| *c as char).collect()
                        } else {
                            String::new()
                        };
                        ServerMessage::Stream {
                            autostart: src[1] == b'1' || src[1] == b'3',
                            format: format,
                            threshold: src[7],
                            output_threshold: src[12],
                            replay_gain: replay_gain,
                            server_port: src[18..20].into_buf().get_u16_be(),
                            server_ip: Ipv4Addr::from(src[20..24].into_buf().get_u32_be()),
                            http_headers: http_headers,
                        }
                    }

                    'q' => ServerMessage::Stop,

                    cmd @ _ => {
                        let mut msg = msg.to_owned();
                        msg.push('_');
                        msg.push(cmd);
                        ServerMessage::Unrecognised(msg)
                    }
                }
            }
            "aude" => ServerMessage::Enable(!(src[1].into_buf().get_u8() == 0)),
            "audg" => ServerMessage::Gain(
                src[10..14].into_buf().get_u32_be() as f64 / GAIN_FACTOR,
                src[14..18].into_buf().get_u32_be() as f64 / GAIN_FACTOR,
            ),
            cmd @ _ => ServerMessage::Unrecognised(cmd.to_owned()),
        }
    }
}
