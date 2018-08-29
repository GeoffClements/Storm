# storm (c) 2018 Geoff Clements <ro2kz0@gmail.com>

## Storm Player
Storm is a music player for the [Slim Protocol](http://wiki.slimdevices.com/index.php/SlimProto_TCP_protocol) which means that it will play music provided by a [Logitech Media Server](https://en.wikipedia.org/wiki/Logitech_Media_Server).

Storm is similar to [squeezelite](https://github.com/ralph-irving/squeezelite) but attempts to reduce (and hopefully simplify) the options as presented by squeezelite.

In addition storm reuses media code in the form of [gstreamer](https://gstreamer.freedesktop.org/) rather than trying to do it all itself. Storm fills the same function as squeezelite but does so in a very different way. Although making a small executable was not a primary aim, an optimised and stripped binary will be less than 3MiB which makes it quite small and so would be suitable to run on small devices.

## Using
To see the run-time options type:
```bash
$ storm -h
```
or for more detail use:
```bash
$ storm --help
```

## Building
Storm is written in Rust and links to the gstreamer library. In order to build you will needed to install the following dependencies:
- ibgstreamer1.0-dev
- libgstreamer-plugins-base1.0-dev
- gstreamer1.0-plugins-base
- gstreamer1.0-plugins-good
- gstreamer1.0-libav
- libgstrtspserver-1.0-dev

Build is simply a case of:
```bash
$ cargo build --release
```
