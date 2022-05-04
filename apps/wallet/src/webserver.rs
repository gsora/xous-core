extern crate simple_server;

use simple_server::{Method, Server, StatusCode};

pub fn run() {
    let host = "0.0.0.0";
    let port = "7878";

    let server = Server::new(|request, mut response| {
        log::info!("Request received. {} {}", request.method(), request.uri());

        match (request.method(), request.uri().path()) {
            (&Method::GET, "/hello") => {
                Ok(response.body("<h1>Hi!</h1><p>Hello from Precursor!</p>".as_bytes().to_vec())?)
            }
            (_, _) => {
                response.status(StatusCode::NOT_FOUND);
                Ok(response.body("<h1>404</h1><p>Not found!<p>".as_bytes().to_vec())?)
            }
        }
    });

    server.listen(host, port);
}