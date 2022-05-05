#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod webserver;
//mod cosmos;
use getrandom::register_custom_getrandom;
use rand_core::RngCore;
use std::cell::RefCell;
use core::fmt::Write;
use graphics_server::api::GlyphStyle;
use graphics_server::{DrawStyle, Gid, PixelColor, Point, Rectangle, TextBounds, TextView};
use num_traits::*;
use std::cell::RefMut;
use locales::t;
#[cfg(feature = "tts")]
use tts_frontend::*;

thread_local! {
    static TRNG_INSTANCE: RefCell<Option<trng::Trng>> = RefCell::new(None);
}

pub fn trng(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    TRNG_INSTANCE.with(|trng_instance| {
        if trng_instance.borrow().is_none() {
            return Err(getrandom::Error::FAILED_RDRAND);
        }

        let mut trng = RefMut::map(
            trng_instance.borrow_mut(), 
            move |v| {
                v.as_mut().unwrap()
        });

        trng.fill_bytes(buf);

        Ok(())
    })
}

register_custom_getrandom!(trng);

/// Basic 'Hello World!' application that draws a simple
/// TextView to the screen.

pub(crate) const SERVER_NAME_WALLET: &str = "_Hello World_";

/// Top level application events.
#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum HelloOp {
    /// Redraw the screen
    Redraw = 0,

    ChangeFocus,

    /// Quit the application
    Quit,
}

struct Hello {
    content: Gid,
    gam: gam::Gam,
    _gam_token: [u32; 4],
    screensize: Point,
    #[cfg(feature = "tts")]
    tts: TtsFrontend,
}

impl Hello {
    fn new(xns: &xous_names::XousNames, sid: xous::SID) -> Self {
        let gam = gam::Gam::new(&xns).expect("Can't connect to GAM");
        let gam_token = gam
            .register_ux(gam::UxRegistration {
                app_name: xous_ipc::String::<128>::from_str(gam::APP_NAME_WALLET),
                ux_type: gam::UxType::Chat,
                predictor: None,
                listener: sid.to_array(),
                redraw_id: HelloOp::Redraw.to_u32().unwrap(),
                gotinput_id: None,
                audioframe_id: None,
                rawkeys_id: None,
                focuschange_id: Some(HelloOp::ChangeFocus.to_u32().unwrap()),
            })
            .expect("Could not register GAM UX")
            .unwrap();

        let content = gam
            .request_content_canvas(gam_token)
            .expect("Could not get content canvas");
        let screensize = gam
            .get_canvas_bounds(content)
            .expect("Could not get canvas dimensions");


        let trng = trng::Trng::new(&xns).unwrap();
        TRNG_INSTANCE.with(|trng_instance| {
            *trng_instance.borrow_mut() = Some(trng);
        });

        Self {
            gam,
            _gam_token: gam_token,
            content,
            screensize,
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

        let mut text_view = TextView::new(
            self.content,
            TextBounds::GrowableFromBr(
                Point::new(
                    self.screensize.x - (self.screensize.x / 2),
                    self.screensize.y - (self.screensize.y / 2),
                ),
                (self.screensize.x / 5 * 4) as u16,
            ),
        );

        text_view.border_width = 1;
        text_view.draw_border = true;
        text_view.clear_area = true;
        text_view.rounded_border = Some(3);
        text_view.style = GlyphStyle::Regular;
        write!(text_view.text, "{}", t!("helloworld.hello", xous::LANG)).expect("Could not write to text view");
        #[cfg(feature="tts")]
        self.tts.tts_simple(t!("helloworld.hello", xous::LANG)).unwrap();

        self.gam
            .post_textview(&mut text_view)
            .expect("Could not render text view");
        self.gam.redraw().expect("Could not redraw screen");
    }
}

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    log::info!("Hello world PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();

    // Register the server with xous
    let sid = xns
        .register_name(SERVER_NAME_WALLET, None)
        .expect("can't register server");

    let mut hello = Hello::new(&xns, sid);

    let modals = modals::Modals::new(&xns).unwrap();

    loop {
        let msg = xous::receive_message(sid).unwrap();
        log::debug!("Got message: {:?}", msg);

        match FromPrimitive::from_usize(msg.body.id()) {
            Some(HelloOp::Redraw) => {
                log::debug!("Got redraw");
                hello.redraw();
            }
            Some(HelloOp::ChangeFocus) => xous::msg_scalar_unpack!(msg, new_state_code, _, _, _, {
                let new_state = gam::FocusState::convert_focus_change(new_state_code);
                match new_state {
                    gam::FocusState::Background => {},
                    gam::FocusState::Foreground => {
                        std::thread::spawn({
                            || {
                                webserver::run();
                            }
                        });
                        log::info!("change focus on wallet");
                        // let tx = cosmos::build_test_tx();

                        // modals.show_notification(
                        //     "Scan this QR code on your smartphone to broadcast the generated transaction.",
                        //     Some(&base64::encode(tx)),
                        // ).unwrap();
                    },
                }
            }),
            Some(HelloOp::Quit) => {
                log::info!("Quitting application");
                break;
            }
            _ => {
                log::error!("Got unknown message");
            }
        }
    }

    log::info!("Quitting");
    xous::terminate_process(0)
}
