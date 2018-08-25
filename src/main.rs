extern crate clap;
#[macro_use]
extern crate log;
extern crate actix;
extern crate bytes;
extern crate fern;
extern crate futures;
extern crate gstreamer as gst;
extern crate mac_address;
extern crate rand;
extern crate thread_control;
extern crate tokio_codec;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_signal;
extern crate tokio_tcp;
extern crate tokio_timer;

mod codec;
mod player;
mod proto;

use log::LevelFilter;

use std::net::Ipv4Addr;
use std::str::FromStr;

const VERSION: &'static str = "0.1.0";

fn main() {
    let opts = clap::App::new("Storm")
        .version(VERSION)
        .author("Geoff Clements <ro2kz0@gmail.com>")
        .about("A Headless Client for Logitec Media Server")
        .arg(
            clap::Arg::with_name("server")
                .short("s")
                .long("server")
                .help("IPv4 address of the Logitec Media Server, otherwise use autodiscovery")
                .takes_value(true)
                .validator(|ip_s| {
                    Ipv4Addr::from_str(&ip_s)
                        .map(|_| ())
                        .map_err(|_| format!("Unable to to parse {}", ip_s))
                }),
        )
        .arg(
            clap::Arg::with_name("log-level")
                .short("d")
                .long("log-level")
                .help("Set logging level")
                .takes_value(true)
                .possible_values(&["info", "warn", "error", "debug", "trace"]),
        )
        .arg(
            clap::Arg::with_name("name")
                .short("n")
                .long("name")
                .help("Set the name of the player")
                .default_value("Storm"),
        )
        .arg(
            clap::Arg::with_name("buffersize")
                .short("b")
                .long("buffersize")
                .help("Input buffer size in KiB")
                .default_value("2048")
                .validator(|bufsize| {
                    bufsize
                        .parse::<u32>()
                        .map(|_| ())
                        .map_err(|_| format!("Unable to to parse {}", bufsize))
                }),
        )
        .get_matches();

    let log_level = match opts.value_of("log-level") {
        Some("error") => LevelFilter::Error,
        Some("warn") => LevelFilter::Warn,
        Some("info") => LevelFilter::Info,
        Some("debug") => LevelFilter::Debug,
        Some("trace") => LevelFilter::Trace,
        _ => LevelFilter::Off,
    };

    // TODO: implement more output options
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}] [{}] {}",
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log_level)
        .chain(std::io::stderr())
        .apply()
        .expect("Failure setting up logger");

    let server_addr = match opts.value_of("server") {
        Some(ip_s) => Ipv4Addr::from_str(ip_s).unwrap(),
        None => proto::discover().unwrap_or_else(|e| {
            error!("Network error whilst looking for server: {}, exiting.", e);
            std::process::exit(1);
        }),
    };

    info!("Using server address: {}", server_addr);
    info!("Name of player is: {}", opts.value_of("name").unwrap());
    info!(
        "Input buffer size is: {} KiB",
        opts.value_of("buffersize").unwrap().parse::<u32>().unwrap()
    );

    proto::run(
        server_addr,
        None,
        opts.value_of("name").unwrap(),
        opts.value_of("buffersize").unwrap().parse::<u32>().unwrap() * 1024,
    );
}
