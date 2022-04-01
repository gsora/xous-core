#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod user_data;
use user_data::*;
mod irc;
use irc::*;
mod repl;
use modals::Modals;
use pddb::Pddb;
use repl::*;
mod cmds;
use cmds::*;
use encoding::all::UTF_8;
use hiirc::*;
use num_traits::*;
use rkyv::*;
use std::{sync::Arc, thread};
use xous_ipc::Buffer;

const ADD_NEW_OPTION: &'static str = "Add new";

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum ReplOp {
    /// a line of text has arrived
    Line = 0, // make sure we occupy opcodes with discriminants < 1000, as the rest are used for callbacks
    /// redraw our UI
    Redraw,
    /// change focus
    ChangeFocus,
    /// exit the application
    Quit,

    MessageReceived,
    MessageSent,
}

// This name should be (1) unique (2) under 64 characters long and (3) ideally descriptive.
pub(crate) const SERVER_NAME_REPL: &str = "_IRC demo application_";

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    log::info!("my PID is {}", xous::process::id());

    let pddb = pddb::Pddb::new();
    load_pddb(&pddb);

    let xns = xous_names::XousNames::new().unwrap();
    // unlimited connections allowed, this is a user app and it's up to the app to decide its policy
    let sid = xns
        .register_name(SERVER_NAME_REPL, None)
        .expect("can't register server");
    // log::trace!("registered with NS -- {:?}", sid);

    let m = modals::Modals::new(&xns).expect("cannot connect to modals server");
    let mut pddb = pddb::Pddb::new();

    let mut connection_modal_shown = false;

    let mut new_message_cid: Option<xous::CID> = None;

    let mut repl = Repl::new(&xns, sid);
    let mut update_repl = true;
    let mut was_callback = false;
    let mut allow_redraw = false;
    loop {
        let msg = xous::receive_message(sid).unwrap();
        log::debug!("got message {:?}", msg);
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(ReplOp::MessageReceived) => {
                let buffer =
                    unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let new_message = buffer
                    .to_original::<NewMessage, _>()
                    .expect("cannot unmarshal new received message");

                match new_message.kind {
                    MessageKind::MOTD => {
                        repl.append_to_first_hist(
                            new_message.content.to_string(),
                            "MOTD".to_string(),
                        );
                    }
                    MessageKind::Message => {
                        repl.circular_push(repl::History {
                            sender: match new_message.sender {
                                Some(msg) => Some(msg.to_string()),
                                None => None,
                            },
                            text: new_message.content.to_string(),
                            is_input: false,
                        });
                    }
                }

                update_repl = true; // set a flag, instead of calling here, so message can drop and calling server is released
                was_callback = false;
            }
            Some(ReplOp::MessageSent) => {}
            Some(ReplOp::Line) => {
                if new_message_cid.is_none() {
                    continue;
                }

                let buffer =
                    unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let s = buffer.as_flat::<xous_ipc::String<4000>, _>().unwrap();
                log::trace!("repl got input line: {}", s.as_str());

                let msg = s.as_str();

                {
                    let msg = NewMessage {
                        kind: MessageKind::Message,
                        sender: None,
                        content: xous_ipc::String::from_str(msg),
                    };

                    let msgbuf = Buffer::into_buf(msg).expect("cannot mutate into buffer");
                    msgbuf
                        .send(
                            new_message_cid.unwrap(),
                            IRCOp::MessageSent.to_u32().unwrap(),
                        )
                        .expect("cannot send new message to repl server");
                }

                repl.circular_push(repl::History {
                    sender: None,
                    text: msg.to_string(),
                    is_input: true,
                });

                update_repl = true; // set a flag, instead of calling here, so message can drop and calling server is released
                was_callback = false;
            }
            Some(ReplOp::Redraw) => {
                if allow_redraw {
                    repl.redraw().expect("REPL couldn't redraw");
                }
            }
            Some(ReplOp::ChangeFocus) => xous::msg_scalar_unpack!(msg, new_state_code, _, _, _, {
                let new_state = gam::FocusState::convert_focus_change(new_state_code);
                match new_state {
                    gam::FocusState::Background => {
                        allow_redraw = false;
                    }
                    gam::FocusState::Foreground => {
                        allow_redraw = true;
                        if !connection_modal_shown {
                            new_message_cid = Some(show_connection_modal(&m, &mut pddb, sid));
                            connection_modal_shown = true
                        }
                    }
                }
            }),
            Some(ReplOp::Quit) => {
                log::error!("got Quit");
                break;
            }
            _ => {
                log::trace!("got unknown message, treating as callback");
                repl.msg(msg);
                update_repl = true;
                was_callback = true;
            }
        }
        if update_repl {
            repl.update(was_callback)
                .expect("REPL had problems updating");
            update_repl = false;
        }
        log::trace!("reached bottom of main loop");
    }
    // clean up our program
    log::error!("main loop exit, destroying servers");
    xns.unregister_server(sid).unwrap();
    xous::destroy_server(sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

fn load_pddb(pddb: &pddb::Pddb) {
    // Since we have to read user profiles, block until we can successfully
    // access PDDB.
    log::debug!("waiting for pddb to be ready...");
    pddb.is_mounted_blocking(None);
    log::debug!("pddb ready, continuing!");
}

fn show_connection_modal(modals: &Modals, pddb: &mut Pddb, callback_sid: xous::SID) -> xous::CID {
    let mut chosen_network: Option<user_data::Network> = None;

    use std::collections::HashMap;

    while chosen_network.is_none() {
        let network_list = user_data::get_networks(pddb).expect("cannot get networks");
        let mut networks_map: HashMap<String, Network> = HashMap::new();

        for network in network_list {
            modals.add_list_item(&network.name).unwrap();
            networks_map.insert(network.name.clone(), network.clone());
        }

        modals.add_list_item(ADD_NEW_OPTION).unwrap();
        let selected_option = modals
            .get_radiobutton("Choose a network to connect to:")
            .unwrap();

        if !selected_option.eq(ADD_NEW_OPTION) {
            chosen_network = Some(networks_map.get(&selected_option).unwrap().clone());
            continue;
        }

        user_data::store_network(new_network_modal(modals), pddb)
            .expect("cannot create new network")
    }

    let chosen_network = chosen_network.unwrap();

    let connection = IRCConnection {
        callback_sid,
        nickname: chosen_network.nickname,
        server: chosen_network.server,
        channel: chosen_network.channel,
        callback_new_message: ReplOp::MessageReceived.to_u32().unwrap(),
    };

    let new_message_sid = connection.connect();

    xous::connect(new_message_sid).expect("cannot connect to irc new message send")
}

fn new_network_modal(modals: &Modals) -> user_data::Network {
    let name = modals
        .get_text("Network name", None, None)
        .expect("cannot show server text box");
    let server = modals
        .get_text("Server address", None, None)
        .expect("cannot show server text box");
    let nickname = modals
        .get_text("Nickname", None, None)
        .expect("cannot show nickname text box");
    let channel = modals
        .get_text("Channel", None, None)
        .expect("cannot show channel text box");

    Network {
        name: name.0.to_string(),
        channel: channel.0.to_string(),
        server: server.0.to_string(),
        nickname: nickname.0.to_string(),
    }
}
