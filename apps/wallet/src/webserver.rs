extern crate simple_server;

use num_traits::*;
use simple_server::Server;
use std::{sync::{Arc, Mutex}};

#[derive(Default)]
struct Webserver {
    authorized: Arc<Mutex<bool>>,

    callback_cid: xous::CID,
    authorization_modal_op: u32,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct AuthorizationResponse {
    authorized: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct AddressbookEntry{
    name: String,
    address: String,
    chain_type: String,
}

impl Webserver {
    fn auth_endpoint(&mut self) -> simple_server::Handler {
        let auth_bool = self.authorized.clone();
        let callback_cid = self.callback_cid.clone();
        let authorization_modal_op = self.authorization_modal_op.to_usize().unwrap();
        Box::new(
            move |_: http::Request<Vec<u8>>,
                  mut response: simple_server::ResponseBuilder|
                  -> simple_server::ResponseResult {
                let mut authorized_value = auth_bool.lock().unwrap();
                match *authorized_value {
                    true => {
                        // already locked
                        return Ok(response.status(403).body(vec![])?);
                    }
                    false => {
                        match xous::send_message(
                            callback_cid,
                            xous::Message::new_blocking_scalar(authorization_modal_op, 0, 0, 0, 0),
                        ) {
                            Ok(xous::Result::Scalar1(authorized)) => {
                                if authorized == 1 {
                                    *authorized_value = true;
                                } else {
                                    return Ok(response.status(403).body(vec![])?);
                                }
                            }
                            _ => {
                                return Ok(response.status(403).body(vec![])?);
                            }
                        };
                    }
                }
                let response_data = serde_json::ser::to_vec(&AuthorizationResponse{authorized: true}).unwrap();
                Ok(response.body(response_data)?)
            },
        )
    }

    fn auth_middleware(&mut self, next_handler: simple_server::Handler) -> simple_server::Handler {
        let auth_bool = self.authorized.clone();

        Box::new(
            move |request: http::Request<Vec<u8>>,
                  mut response: simple_server::ResponseBuilder|
                  -> simple_server::ResponseResult {
                match *auth_bool.lock().unwrap() {
                    true => next_handler(request, response),
                    false => Ok(response.status(403).body(vec![])?),
                }
            },
        )
    }

    fn addressbook(&mut self) -> simple_server::Handler {
        Box::new(
            move |_: http::Request<Vec<u8>>,
                  mut response: simple_server::ResponseBuilder|
                  -> simple_server::ResponseResult {

                    let ae = AddressbookEntry {
                        name:"name".to_string(),
                        address: "address".to_string(),
                        chain_type: "chain".to_string(),
                    };

                    let bytes = serde_json::ser::to_vec(&ae).unwrap();

                Ok(response.status(200).body(bytes)?)
            },
        )
    }

    fn withdraw_authorization(&mut self) {
        *self.authorized.lock().unwrap() = false;
    }
}

pub fn run(cid: xous::CID, auth_callback: u32) {
    let host = "0.0.0.0";
    let port = "80";

    let mut server = Server::new();

    let mut w = Webserver::default();
    w.authorization_modal_op = auth_callback;
    w.callback_cid = cid;

    server.route(http::Method::GET, "/authorize", w.auth_endpoint());

    let addressbook_handler = w.addressbook();
    server.route(
        http::Method::GET,
        "/addressbook",
        w.auth_middleware(addressbook_handler),
    );

    server.listen(host, port);
}
