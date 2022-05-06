use core::{cell::{RefCell}};
use std::{sync::Mutex, sync::Arc, net::TcpStream, io::Write, thread};
use num_traits::*;
use xous_ipc::Buffer;
use once_cell::sync::Lazy;

#[allow(dead_code)]
static CONNECTED: Lazy<Mutex<RefCell<bool>>> = Lazy::new(|| {
    Mutex::new(RefCell::new(false))
});

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
                Err(error) => {}, // TODO: handle disconnection
                Ok(stream) => {
                    let mut shared_stream = shared_stream.lock().unwrap();
                    *shared_stream = Some(stream);
                    CONNECTED.lock().unwrap().replace(true);
                }
            }
        }
    }


    // loop {
    //     let msg = xous::receive_message(sid).unwrap();
    //     match FromPrimitive::from_usize(msg.body.id()) {
    //         Some(TcpLoggerOp::Log) => {
    //             if shared_stream_copy.lock().unwrap().is_none() {
    //                 continue
    //             }

    //             let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
    //             let s = buffer.as_flat::<xous_ipc::String<4000>, _>().unwrap(); // 4k chars ought be enough for everybody
    //             let mut shared_stream =  shared_stream_copy.lock().unwrap();
    //             match &mut *shared_stream {
    //                 Some(stream) => {
    //                     stream.write(s.as_str().as_bytes()).unwrap();
    //                     stream.flush().unwrap();
    //                 },
    //                 None => {},
    //             }                
    //         },
    //         _ => {}, // TODO: how to unwrap data?
    //     }
    // }
}

pub fn write_str(data: &str) {
    if !*CONNECTED.lock().unwrap().borrow() {
        return
    }
    let mut shared_stream =  SHARED_STREAM.lock().unwrap();
    match &mut *shared_stream {
        Some(stream) => {
            stream.write_all(data.as_bytes()).unwrap();
            stream.flush().unwrap();
        },
        None => {},
    }     
}

pub fn remote_putc(c: u8) {
    if !*CONNECTED.lock().unwrap().borrow() {
        return
    }

    let mut shared_stream =  SHARED_STREAM.lock().unwrap();
    match &mut *shared_stream {
        Some(stream) => {
            let vc = [c];
            stream.write(&vc).unwrap();
            stream.flush().unwrap();
        },
        None => {},
    }           
}