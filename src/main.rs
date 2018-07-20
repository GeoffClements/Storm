extern crate clap;
#[macro_use] extern crate log;
extern crate fern;
extern crate tokio_core;
extern crate tokio_timer;
extern crate tokio_codec;
extern crate tokio_io;
extern crate tokio_tcp;
extern crate futures;
extern crate bytes;
extern crate actix;

mod proto;
mod codec;

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
                .possible_values(&["error", "warn", "info", "debug", "trace"]),
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

    info!("Using server address {}", server_addr);

    proto::run(server_addr);
}
