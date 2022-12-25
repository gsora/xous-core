#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use padme_core::default::{NoScreen, NoSerial, NoSpeaker};
use padme_core::{FRAME_WIDTH, FRAME_HEIGHT};
use core::fmt::Write;
use gam::{Bitmap, Img};
use graphics_server::api::GlyphStyle;
use graphics_server::{DrawStyle, Gid, PixelColor, Point, Rectangle, TextBounds, TextView};
use locales::t;
use num_traits::*;
use padme_core::Rom;
use padme_core::Screen;
use rgy::{VRAM_HEIGHT, VRAM_WIDTH};
#[cfg(feature = "tts")]
use tts_frontend::*;

/// Basic 'Hello World!' application that draws a simple
/// TextView to the screen.

pub(crate) const SERVER_NAME_GB: &str = "_GameBoy emulator_";

/// Top level application events.
#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum GbOp {
    /// Redraw the screen
    Redraw = 0,

    FocusChange = 1,

    /// Quit the application
    Quit,
}

struct Gb {
    content: Gid,
    gam: gam::Gam,
    _gam_token: [u32; 4],
    screensize: Point,
    emu_running: bool,
    #[cfg(feature = "tts")]
    tts: TtsFrontend,
}

impl Gb {
    fn new(xns: &xous_names::XousNames, sid: xous::SID) -> Self {
        let gam = gam::Gam::new(&xns).expect("Can't connect to GAM");
        let gam_token = gam
            .register_ux(gam::UxRegistration {
                app_name: xous_ipc::String::<128>::from_str(gam::APP_NAME_GB),
                ux_type: gam::UxType::Chat,
                predictor: None,
                listener: sid.to_array(),
                redraw_id: GbOp::Redraw.to_u32().unwrap(),
                gotinput_id: None,
                audioframe_id: None,
                rawkeys_id: None,
                focuschange_id: Some(GbOp::FocusChange.to_u32().unwrap()),
            })
            .expect("Could not register GAM UX")
            .unwrap();

        let content = gam
            .request_content_canvas(gam_token)
            .expect("Could not get content canvas");
        let screensize = gam
            .get_canvas_bounds(content)
            .expect("Could not get canvas dimensions");
        Self {
            gam,
            _gam_token: gam_token,
            content,
            screensize,
            emu_running: false,
            #[cfg(feature = "tts")]
            tts: TtsFrontend::new(xns).unwrap(),
        }
    }

    /// Clear the entire screen.
    fn clear_area(&self) {
        self.gam
            .draw_rectangle(
                self.content,
                Rectangle::new_with_style(
                    Point::new(0, 0),
                    self.screensize,
                    DrawStyle {
                        fill_color: Some(PixelColor::Light),
                        stroke_color: None,
                        stroke_width: 0,
                    },
                ),
            )
            .expect("can't clear content area");
    }

    /// Redraw the text view onto the screen.
    fn redraw(&mut self) {
        self.clear_area();

        if !self.emu_running {
            // let cfg = rgy::Config::new();
            let rom = include_bytes!("../blue.gb");

            // let xns = xous_names::XousNames::new().unwrap();
            // let gbe = GbEmu::new(self.content.clone(), &xns);
            // rgy::run(cfg, rom, gbe);
            self.emu_running = true;
            let mut rom = match Rom::load(rom.to_vec()) {
                Ok(rom) => rom,
                Err(error) => panic!("rom loading error")
            };
            
            let gidclone = self.content.clone();
            
            std::thread::spawn(move || {
                let xns = xous_names::XousNames::new().unwrap();
                let gbe = GbEmu::new(gidclone, &xns);
                let mut emulator = padme_core::System::new(rom, gbe, NoSerial, NoSpeaker);
                // Set the number of frame per seconds
                // This also sets the number of cycles needed per frame given the fixed CPU clock frequency
                emulator.set_frame_rate(60);
                
                loop {
                    // We need to know how much time it took to display a frame
                    let t0 = std::time::Instant::now();
                    // This executes all the cycles needed to display one frame
                    emulator.update_frame();
                    
                    // Now we just need to wait the remaining time before the next frame
                    // This is because we need to keep ~60 frames / second
                    let frame_time = t0.elapsed();
                    let min_frame_time = emulator.min_frame_time();
                    if frame_time < min_frame_time {
                        std::thread::sleep(min_frame_time - frame_time);
                    }
                }
            });
        }
    }
}

struct GbEmu {
    gam: gam::Gam,
    content: Gid,
    render_cid: xous::CID,
    fb: Vec<u8>,
}

impl GbEmu {
    fn new(gid: Gid, xns: &xous_names::XousNames) -> GbEmu {
        let server = xous::create_server().unwrap();
        GbEmu {
            gam: gam::Gam::new(xns).unwrap(),
            content: gid,
            render_cid: xous::connect(server).unwrap(),
            fb: vec![0; FRAME_WIDTH * FRAME_HEIGHT * 3],
        }
    }
}

impl padme_core::Screen for GbEmu {
    fn set_pixel(&mut self, px: &padme_core::Pixel, x: u8, y: u8) {
        let i = (x as usize + y as usize * FRAME_WIDTH) * 3;
        self.fb[i] = px.r;
        self.fb[i + 1] = px.g;
        self.fb[i + 2] = px.b;
    }

    fn update(&mut self) {
        let img = Img::new(self.fb.clone(), VRAM_WIDTH, gam::PixelType::U8x3);
        let bitmap = Bitmap::from_img(
            &img,
            None,
            //Some(Point::new(VRAM_WIDTH as i16, VRAM_HEIGHT as i16)),
        );

        self.gam.draw_bitmap(self.content, &bitmap);
    }
}

impl rgy::Hardware for GbEmu {
    fn vram_update(&mut self, line: usize, buffer: &[u32]) {
        // let raw_bytes: Vec<u8> = buffer.iter().fold(vec![], |mut ret, elem| {
        //     let r = (elem & 0xff) as u8; // r
        //     let g = ((elem >> 8) & 0xff) as u8; // g
        //     let b = ((elem >> 16) & 0xff) as u8; // b
        //     ret.push(r);
        //     ret.push(g);
        //     ret.push(b);

        //     ret
        // });

        // self.fb[line] = raw_bytes;

        // let raw_fb: Vec<u8> = self.fb.iter().fold(vec![], |mut ret, elem| {
        //     ret.extend(elem);
        //     ret
        // });

        // let img = Img::new(raw_fb, VRAM_WIDTH, gam::PixelType::U8x3);
        // //log::info!("image width: {}, height: {}", img.width(), img.height());
        // let bitmap = Bitmap::from_img(
        //     &img,
        //     Some(Point::new(VRAM_WIDTH as i16, VRAM_HEIGHT as i16)),
        // );

        // self.gam.draw_bitmap(self.content, &bitmap);
    }

    fn joypad_pressed(&mut self, key: rgy::Key) -> bool {
        //log::info!("called joypad_pressed, key: {:?}", key);
        false
    }

    fn sound_play(&mut self, stream: Box<dyn rgy::Stream>) {
        //log::info!("called sound_play");
    }

    fn clock(&mut self) -> u64 {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Couldn't get epoch");
        epoch.as_micros() as u64
    }

    fn send_byte(&mut self, b: u8) {
        //log::info!("called send_byte");
    }

    fn recv_byte(&mut self) -> Option<u8> {
        //log::info!("called recv_byte");
        None
    }

    fn load_ram(&mut self, size: usize) -> Vec<u8> {
        //log::info!("called load_ram");
        vec![0; size]
    }

    fn save_ram(&mut self, ram: &[u8]) {
        //log::info!("called save_ram");
    }
}

fn main() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("GameBoy emulator PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();

    // Register the server with xous
    let sid = xns
        .register_name(SERVER_NAME_GB, None)
        .expect("can't register server");

    let mut hello = Gb::new(&xns, sid);

    loop {
        let msg = xous::receive_message(sid).unwrap();
        log::debug!("Got message: {:?}", msg);

        match FromPrimitive::from_usize(msg.body.id()) {
            Some(GbOp::Redraw) => {
                log::debug!("Got redraw");
                hello.redraw();
            }
            Some(GbOp::Quit) => {
                log::info!("Quitting application");
                break;
            }
            Some(GbOp::FocusChange) => xous::msg_scalar_unpack!(msg, new_state_code, _, _, _, {
                let new_state = gam::FocusState::convert_focus_change(new_state_code);
                match new_state {
                    gam::FocusState::Background => {}
                    gam::FocusState::Foreground => {}
                }
            }),
            _ => {
                log::error!("Got unknown message");
            }
        }
    }

    log::info!("Quitting");
    xous::terminate_process(0)
}
