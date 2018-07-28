use actix;
use gst;

pub enum PlayerControl {
    Gain(f64, f64),
    Enable(bool),
}

impl actix::Message for PlayerControl {
    type Result = ();
}

pub struct Player {
    gain: f64,
    enable: bool,
}

impl Player {
    pub fn new() -> Self {
        Player {
            gain: 1.0,
            enable: false,
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
                self.gain = if gain_left > gain_right { gain_left } else { gain_right };
                info!("Setting gain to {}", self.gain);
            }
            PlayerControl::Enable(enable) => {
                info!("Setting enable to {}", enable);
                self.enable = enable;
            }
        }
    }
}
