use actix;
use actix::{Actor, ActorContext, Arbiter, AsyncContext, Context, System};
use futures::{future, Future, Sink, Stream};
use rand::{thread_rng, Rng};
use tokio_codec::FramedRead;
use tokio_core;
use tokio_io::io::WriteHalf;
use tokio_io::AsyncRead;
use tokio_tcp::TcpStream;
use tokio_timer;

use codec;

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

struct Proto {
    sync_group_id: Option<String>,
    framed: actix::io::FramedWrite<WriteHalf<TcpStream>, codec::SlimCodec>,
}

impl Actor for Proto {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Context<Self>) {
        let caps = "Model=Storm,AccuratePlayPoints=1,HasDigitalOut=1,HasPolarityInversion=1";
        let caps = match self.sync_group_id {
            Some(ref sync_group) => {
                let sg = format!(",SyncgroupID={}", sync_group);
                format!("{}{}", caps, sg)
            }
            None => caps.to_owned(),
        };

        let helo = codec::ClientMessage::Helo {
            device_id: 12,
            revision: 0,
            mac: random_mac(),
            uuid: [0; 16],
            wlan_channel_list: 0,
            bytes_received: 0,
            capabilities: caps,
        };

        info!("Sending Helo");
        self.framed.write(helo);
    }

    fn stopping(&mut self, _ctx: &mut Context<Self>) -> actix::Running {
        info!("Sending Bye");
        self.framed.write(codec::ClientMessage::Bye(0));
        actix::Running::Stop
    }
}

impl actix::io::WriteHandler<io::Error> for Proto {}

impl actix::StreamHandler<codec::ServerMessage, io::Error> for Proto {
    fn handle(&mut self, msg: codec::ServerMessage, ctx: &mut Context<Self>) {
        match msg {
            codec::ServerMessage::Serv {
                ip_address,
                sync_group_id,
            } => {
                info!("Got serv message");
                spawn(ip_address, sync_group_id);
                ctx.stop();
            }
            codec::ServerMessage::Unrecognised(msg) => {
                warn!("Unrecognised message: {}", msg);
            }
            _ => {}
        }
    }
}

pub fn run(server_ip: Ipv4Addr, sync_group: Option<String>) {
    let sys = System::new("Storm");
    spawn(server_ip, sync_group);
    sys.run();
}

fn spawn(server_ip: Ipv4Addr, sync_group: Option<String>) {
    let addr = SocketAddr::new(IpAddr::V4(server_ip), 3483);
    Arbiter::spawn(
        TcpStream::connect(&addr)
            .and_then(move |stream| {
                Proto::create(move |ctx| {
                    let (r, w) = stream.split();
                    ctx.add_stream(FramedRead::new(r, codec::SlimCodec));
                    Proto {
                        sync_group_id: sync_group,
                        framed: actix::io::FramedWrite::new(w, codec::SlimCodec, ctx),
                    }
                });
                future::ok(())
            })
            .map_err(|e| {
                error!("Cannot connect to server: {}", e);
                ::std::process::exit(2)
            }),
    );
}

struct Discover;

impl tokio_core::net::UdpCodec for Discover {
    type In = Ipv4Addr;
    type Out = char;

    fn decode(&mut self, src: &SocketAddr, _buf: &[u8]) -> io::Result<Self::In> {
        if let SocketAddr::V4(addr) = src {
            Ok(*addr.ip())
        } else {
            unreachable!()
        }
    }

    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> SocketAddr {
        buf.push(msg as u8);
        "255.255.255.255:3483".parse().unwrap()
    }
}

pub fn discover() -> io::Result<Ipv4Addr> {
    let mut core = tokio_core::reactor::Core::new()?;
    let handle = core.handle();

    let sock = tokio_core::net::UdpSocket::bind(&"0.0.0.0:0".parse().unwrap(), &handle)?;
    sock.set_broadcast(true)?;

    let (discover_out, discover_in) = sock.framed(Discover).split();

    info!("Looking for server ...");

    let pings = tokio_timer::Interval::new(Instant::now(), Duration::from_secs(5))
        .map(|_| 'e')
        .map_err(|_| ());
    let pinger = discover_out
        .sink_map_err(|_| ())
        .send_all(pings)
        .map(|_| ())
        .map_err(|_| ());
    handle.spawn(pinger);

    let discovery = discover_in.take(1).into_future();
    match core.run(discovery).map_err(|(e, _)| e) {
        Ok((Some(addr), _)) => {
            info!("Found server at {}", addr);
            Ok(addr)
        }
        Err(e) => Err(e),
        _ => unreachable!(),
    }
}

fn random_mac() -> [u8; 6] {
    let mut rng = thread_rng();
    let mut mac = [0; 6];
    let mut mac_temp = Vec::new();

    (0..6).for_each(|_| mac_temp.push(rng.gen::<u8>()));
    mac_temp[0] |= 0b0000_0010;
    mac.copy_from_slice(&mac_temp);
    mac
}
