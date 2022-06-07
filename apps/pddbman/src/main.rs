#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use core::fmt::Write;
use std::collections::HashMap;
use graphics_server::api::GlyphStyle;
use graphics_server::{DrawStyle, Gid, PixelColor, Point, Rectangle, TextBounds, TextView};
use num_traits::*;
use std::io::Result;

pub(crate) const SERVER_NAME_PDDBMAN: &str = "_PDDB manager_";

/// Top level application events.
#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum PddbManOp {
    /// Redraw the screen
    Redraw = 0,

    FocusChange,
    RawKey,

    /// Quit the application
    Quit,
}

struct PddbMan {
    content: Gid,
    gam: gam::Gam,
    _gam_token: [u32; 4],
    screensize: Point,

    pddb: pddb::Pddb,
    modals: modals::Modals,
}

impl PddbMan {
    fn new(xns: &xous_names::XousNames, sid: xous::SID) -> Self {
        let gam = gam::Gam::new(&xns).expect("Can't connect to GAM");
        let gam_token = gam
            .register_ux(gam::UxRegistration {
                app_name: xous_ipc::String::<128>::from_str(gam::APP_NAME_PDDBMAN),
                ux_type: gam::UxType::Framebuffer,
                predictor: None,
                listener: sid.to_array(),
                redraw_id: PddbManOp::Redraw.to_u32().unwrap(),
                gotinput_id: None,
                audioframe_id: None,
                rawkeys_id: Some(PddbManOp::RawKey.to_u32().unwrap()),
                focuschange_id: Some(PddbManOp::FocusChange.to_u32().unwrap()),
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
            pddb: pddb::Pddb::new(),
            modals: modals::Modals::new(&xns).unwrap(),
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
    fn redraw(&mut self, basis_name: String) {
        self.clear_area();

        let mut text_view = TextView::new(
            self.content,
            TextBounds::GrowableFromBr(
                Point::new(
                    self.screensize.x - (self.screensize.x / 5),
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
        write!(
            text_view.text,
            "Basis currently unlocked: {basis_name}\nPress any button to bring up menu.",
        )
        .expect("Could not write to text view");

        self.gam
            .post_textview(&mut text_view)
            .expect("Could not render text view");
        self.gam.redraw().expect("Could not redraw screen");
    }

    fn menu_items_mappings(&self) -> Vec<&str> {
        vec![
            "Unlock basis",
            "Create new basis",
            "Delete basis",
            "Lock basis",
            "Close menu",
        ]
    }

    fn show_menu(&self) {
        self.modals.add_list(self.menu_items_mappings()).unwrap();

        self.modals.get_radiobutton("Select an action:").unwrap();

        let idx = self.modals.get_radio_index().unwrap();
        log::trace!("selected radio index: {idx}");

        match idx {
            0 => self
                .unlock_basis()
                .unwrap_or_else(|err| self.show_error_notification(err)),
            1 => self
                .create_basis()
                .unwrap_or_else(|err| self.show_error_notification(err)),
            2 => self
                .delete_basis()
                .unwrap_or_else(|err| self.show_error_notification(err)),
            3 => self
                .lock_basis()
                .unwrap_or_else(|err| self.show_error_notification(err)),
            4 => {}
            _ => {}
        };
    }

    fn modal_basis_name(&self) -> String {
        let basis_name = self
            .modals
            .alert_builder("Basis name?")
            .field(None, None)
            .build()
            .unwrap();

        basis_name.first().as_str().to_string()
    }

    fn show_error_notification(&self, err: std::io::Error) {
        self.modals
            .show_notification(&format!("Cannot execute PDDB action: {}", err), None)
            .unwrap();
    }

    fn lock_basis(&self) -> Result<()> {
        self.pddb.lock_basis(&self.modal_basis_name())
    }

    fn unlock_basis(&self) -> Result<()> {
        self.pddb.unlock_basis(&self.modal_basis_name(), None)
    }

    fn create_basis(&self) -> Result<()> {
        self.pddb.create_basis(&self.modal_basis_name())
    }

    fn delete_basis(&self) -> Result<()> {
        self.pddb.delete_basis(&self.modal_basis_name())
    }
}

fn main() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    log::info!("PDDB manager PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();

    // Register the server with xous
    let sid = xns
        .register_name(SERVER_NAME_PDDBMAN, None)
        .expect("can't register server");

    let mut pddb_man = PddbMan::new(&xns, sid);

    loop {
        let msg = xous::receive_message(sid).unwrap();
        log::debug!("Got message: {:?}", msg);

        match FromPrimitive::from_usize(msg.body.id()) {
            Some(PddbManOp::Redraw) => {
                log::debug!("Got redraw");
                let open_basis = pddb_man.pddb.latest_basis().unwrap_or("none".to_string());
                pddb_man.redraw(open_basis);
            }
            Some(PddbManOp::Quit) => {
                log::info!("Quitting application");
                break;
            }
            Some(PddbManOp::FocusChange) => {
                log::trace!("got focuschange");
            }
            Some(PddbManOp::RawKey) => {
                log::trace!("Got rawkeys");
                pddb_man.show_menu();
            }
            _ => {
                log::error!("Got unknown message");
            }
        }
    }

    log::info!("Quitting");
    xous::terminate_process(0)
}
