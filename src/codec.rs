use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use tokio_codec;

use std::convert::From;
use std::io;
use std::net::Ipv4Addr;

struct SlimCodec;

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

        match ServerMessage::from(msg) {
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
        mac: [u8; 6],
        uuid: [u8; 16],
        wlan_channel_list: u16,
        bytes_received: u64,
        // language: u16,
        capabilities: String,
    },
}

enum ServerMessage {
    Serv {
        ip_address: Ipv4Addr,
        sync_group_id: Option<String>,
    },
    Unrecognised(String),
    Error,
}

impl From<ClientMessage> for BytesMut {
    fn from(src: ClientMessage) -> BytesMut {
        let mut buf = Vec::with_capacity(512);

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
                buf.put(mac.as_ref());
                buf.put(uuid.as_ref());
                buf.put_u16_be(wlan_channel_list);
                buf.put_u64_be(bytes_received);
                buf.put(capabilities.as_bytes());
            }
        }

        let mut msg_length = Vec::new();
        msg_length.put_u32_be(buf.len() as u32 + 4);
        msg_length.into_iter().rev().for_each(|v| buf.insert(4, v));
        BytesMut::from(buf)
    }
}

impl From<BytesMut> for ServerMessage {
    fn from(mut src: BytesMut) -> ServerMessage {
        let cmd: String = src.split_to(4).into_iter().map(|c| c as char).collect();

        match cmd.as_str() {
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
            cmd @ _ => ServerMessage::Unrecognised(cmd.to_owned()),
        }
    }
}
