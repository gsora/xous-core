use std::{sync::Mutex, sync::Arc, net::TcpStream};
use std::io::Write;

#[derive(Clone)]
pub struct TcpLogger {
    stream: Arc<Mutex<Option<TcpStream>>>,
}

#[cfg(feature = "tcp_logging")]
impl TcpLogger {
    
    pub fn new() -> Self {
        let tl = TcpLogger {
            stream: Arc::new(Mutex::new(None)),
        };

        std::thread::spawn({
            let mut tl = tl.clone();
            move || {
                tl.run_thread()
            }
        });

        tl
    }

    fn run_thread(&mut self) -> ! {    
        loop {
            let listener = std::net::TcpListener::bind("0.0.0.0:65535");
            let listener = match listener {
                Ok(listener) => listener,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    continue;
                },
            };
    
            for i in listener.incoming() {
                match i {
                    Err(error) => { // TODO: how do we print this without logging to socket?
                        let mut shared_stream = self.stream.lock().unwrap();
                        let shared_stream_inner = shared_stream.as_ref();
                        match shared_stream_inner {
                            Some(ss) => {
                                drop(ss);
                            }
                            None => {
                                continue;
                            }
                        }
                        *shared_stream = None;
                    }, 
                    Ok(stream) => {
                        let mut shared_stream = self.stream.lock().unwrap();
                        *shared_stream = Some(stream);
                    }
                }
            }
        }
    }

    pub fn write_str(&self, data: &str) {
        let mut shared_stream =  self.stream.lock().unwrap();
        match &mut *shared_stream {
            Some(stream) => {
                stream.write_all(data.as_bytes()).unwrap();
                stream.flush().unwrap();
            },
            None => {},
        }     
    }
    
    pub fn write_array(&self, data: &[u8]) {
        let mut shared_stream =  self.stream.lock().unwrap();
        match &mut *shared_stream {
            Some(stream) => {
                stream.write_all(data).unwrap();
                stream.flush().unwrap();
            },
            None => {},
        }     
    }
}

#[cfg(not(feature = "tcp_logging"))]
impl TcpLogger {
    pub fn new() -> Self {
        TcpLogger {
            stream: Arc::new(Mutex::new(None))
        }
    }

    pub fn write_str(&self, data: &str) {
        // no-op
    }
    
    pub fn write_array(&self, data: &[u8]) {
        // no-op
    }
}