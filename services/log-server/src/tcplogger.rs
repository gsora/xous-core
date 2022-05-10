use core::{cell::{RefCell}};
use std::{sync::Mutex, sync::Arc, net::TcpStream, io::Write, thread};
use num_traits::*;
use xous_ipc::Buffer;
use once_cell::sync::Lazy;
use threadpool::ThreadPool;

#[allow(dead_code)]
static SHARED_STREAM: Lazy<Arc<Mutex<Option<TcpStream>>>> = Lazy::new(|| {
    Arc::new(Mutex::new(None))
});


#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum TcpLoggerOp {
    Log = 0
}

pub fn start() {
    std::thread::spawn(move || run_thread());
    // std::thread::spawn(|| {
    //     loop {
    //         write_str("hello!\n");
    //         std::thread::sleep(std::time::Duration::from_millis(1000));
    //     }
    // });
}

fn run_thread() -> ! {
    let shared_stream = SHARED_STREAM.clone();
    let pool = ThreadPool::new(2);

    loop {

        let listener = std::net::TcpListener::bind("0.0.0.0:3333");
        let listener = match listener {
            Ok(listener) => listener,
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                continue;
            },
        };

        for i in listener.incoming() {
            match i {
                Err(error) => {

                }, // TODO: handle disconnection
                Ok(stream) => {
                    let mut shared_stream = shared_stream.lock().unwrap();
                    if shared_stream.is_some() {
                        stream.shutdown(std::net::Shutdown::Both);
                        std::thread::sleep_ms(150);
                        drop(stream);
                        continue
                    }

                    *shared_stream = Some(stream);
                }
            }
        }
    }
}

pub fn write_str(data: &str) {
    let mut shared_stream =  SHARED_STREAM.lock().unwrap();
    match &mut *shared_stream {
        Some(stream) => {
            let mut was_error = false;
            if stream.write_all(data.as_bytes()).is_err() || stream.flush().is_err() {
                was_error = true;
            }
        
            if was_error {
                stream.shutdown(std::net::Shutdown::Both);
                std::thread::sleep_ms(150);
                drop(stream);
                *shared_stream = None;
                return
            }

            //std::thread::sleep_ms(50);
        },
        None => {},
    }     
}

pub fn write_array(data: &[u8]) {
    let mut shared_stream =  SHARED_STREAM.lock().unwrap();
    match &mut *shared_stream {
        Some(stream) => {
            let mut was_error = false;
            if stream.write_all(data).is_err() || stream.flush().is_err() {
                was_error = true;
            }
        
            if was_error {
                stream.shutdown(std::net::Shutdown::Both);
                std::thread::sleep_ms(150);
                drop(stream);
                *shared_stream = None;
                return
            }

            //std::thread::sleep_ms(50);
        },
        None => {},
    }     
}

pub fn remote_putc(c: u8) {
    let mut shared_stream =  SHARED_STREAM.lock().unwrap();
    match &mut *shared_stream {
        Some(stream) => {
            let vc = [c];
            let mut was_error = false;
            if stream.write(&vc).is_err() || stream.flush().is_err() {
                was_error = true;
            }
        
            if was_error {
                stream.shutdown(std::net::Shutdown::Both);
                std::thread::sleep_ms(150);
                drop(stream);
                *shared_stream = None;
                 return
            }

            //std::thread::sleep_ms(50);
        },
        None => {},
    }           
}