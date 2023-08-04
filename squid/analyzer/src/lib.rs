mod mode;
mod buffer;

use mode::Mode;
use buffer::Buffer;

use log::{LevelFilter, info, error};
use std::boxed::Box;
use std::collections::HashMap;
use std::convert::From;
use std::ffi::{c_void, c_char, CStr};
use std::io::prelude::*;
use std::ptr::null;
use std::sync::Once;
use syslog::{Logger, LoggerBackend, Facility, Formatter3164, BasicLogger};

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

const OUTPUT_BUFFER_SIZE: usize = 1024 * 1024;

fn setup_hooks() {
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            info!("Hooked panic with massage: {}", message);
        }

        if let Some(location) = panic_info.location() {
            info!("panic occurred in file '{}' at line {}",
                     location.file(),
                     location.line(),
                     );
        } else {
            info!("panic occurred but can't get location information...");
        }
        panic_hook(panic_info);
    }));
}

trait Instance<T> {
    fn new() -> Option<T>;
}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}

struct Buffers {
    responses: HashMap<i64, Buffer>,
    headers: HashMap<i64, HashMap<String, String>>,
}

impl Instance<Buffers> for Buffers {
    fn new() -> Option<Buffers> {
        Some(Buffers { responses: HashMap::new(), headers: HashMap::new() })
    }
}

fn get_buffers() -> &'static mut Buffers {
    unsafe {
        ONCE_BUFFERS.call_once(|| {
            BUFFERS = Buffers::new();
        });
        match & mut BUFFERS {
            Some(b) => b,
            None => panic!("Buffers not available"),
        }
    }
}

fn append(id: i64, chunk: *const c_void, size: usize) {
    let ptr = chunk as *const u8;
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => unsafe {
            buffer.write_bytes(std::slice::from_raw_parts(ptr, size));
        },
        None => {
            panic!("Unexpected condition: asking to transfer data to uninitialized transaction with id {}", id);
        },
    };
}

/*
fn brotli_decompress(buffer: &[u8]) -> Vec<u8> {
    let mut decompressor = brotli_decompressor::Decompressor::new(buffer, buffer.len());
    let mut decoded = Vec::new();
    decompressor.read_to_end(&mut decoded).unwrap();
    decoded
}
*/

fn transform(bytes: usize, content: &mut [u8] ) -> Chunk {
    Chunk {
        size: bytes,
        bytes: content.as_ptr() as *const c_void,
    }
    //Chunk { size: 0, bytes: null(), }
}

#[no_mangle]
pub extern "C" fn uri(id: i64, uri_str: *const c_char, mode: i64, method_str: *const c_char) {
    let uri = unsafe {CStr::from_ptr(uri_str)}.to_str().unwrap().to_owned();
    let method = unsafe {CStr::from_ptr(method_str)}.to_str().unwrap().to_owned();
    let buffers = get_buffers();
    let encoding = match buffers.headers.get(&id) {
        Some(headers) => headers.get("Content-Encoding"),
        _ => None,
    };
    buffers.responses.insert(id, Buffer::new(id, uri.to_string(), encoding));

    info!("Transaction {} initialized with mode {} for {} uri {}", id, Mode::from(mode), method, uri);
}

#[no_mangle]
pub extern "C" fn send(id: i64, _offset: usize, _size: usize) -> Chunk {
    //const MIN : usize = 1024;
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => {
            match &buffer.encoding {
                Some(encoding) => {
                    if encoding != "gzip" {
                       match buffer.bytes_receiver.try_recv() {
                           Ok(bytes) => {
                               buffer.transfer_chunk = bytes;
                               return transform(buffer.transfer_chunk.len(), &mut buffer.transfer_chunk);
                           },
                           Err(_) => {
                               return Chunk { size: 0, bytes: null() };
                           }
                       }
                    }
                },
                None => {
                    match buffer.bytes_receiver.try_recv() {
                        Ok(bytes) => {
                            buffer.transfer_chunk = bytes;
                            return transform(buffer.transfer_chunk.len(), &mut buffer.transfer_chunk);
                        },
                        Err(_) => {
                            return Chunk { size: 0, bytes: null() };
                        }
                    }
                }
            }

            if buffer.error {
                return Chunk {
                    size:0, bytes: null(),
                };
            }

            let mut output_buffer : [u8; OUTPUT_BUFFER_SIZE ] = [0; OUTPUT_BUFFER_SIZE];
            let result = {
                if buffer.is_done {
                    buffer.encoder.finish(&mut output_buffer)
                } else {
                    buffer.encoder.read(&mut output_buffer)
                }
            };

            let bytes = match  result {
                Ok(bytes) => { bytes },
                Err(e) => {
                    error!("Failed reading for id {} (uri: {}). Will return 0 bytes. Error: {}", buffer.id, buffer.uri, e);
                    buffer.error = true;
                    0
                }
            };

            buffer.transfer_chunk = output_buffer[0..bytes].to_vec();
            transform(bytes, buffer.transfer_chunk.as_mut_slice())
        },
        None => {
            Chunk { size: 0, bytes: null() }
        }
    }
}

#[no_mangle]
pub extern "C" fn receive(id: i64, chunk: *const c_void, size: usize) {
    append(id, chunk, size);
}

#[no_mangle]
pub extern "C" fn cleanup(id: i64) {
    let buffers = get_buffers();
    match buffers.responses.remove(&id) {
        Some(buffer) => {
            drop(buffer);
        },
        None => (),
    };

    match buffers.headers.remove(&id) {
         Some(headers) => {
            drop(headers);
        },
        None => (),
    };

    info!("{} & {} transactions currently active. Capacities @ {} & {}",
      buffers.responses.len(), buffers.headers.len(), buffers.responses.capacity(), buffers.headers.capacity(),
    );
}

#[no_mangle]
pub extern "C" fn header(id: i64, name: *const c_char, value: *const c_char) {
    let name = unsafe {CStr::from_ptr(name)}.to_str().unwrap().to_owned();
    let value = unsafe {CStr::from_ptr(value)}.to_str().unwrap().to_owned();
    let buffers = get_buffers();
    match buffers.headers.get_mut(&id) {
        Some(headers) => {
            headers.insert(name.clone(), value.clone());
        },
        None => {
            let mut headers = HashMap::new();
            headers.insert(name.clone(), value.clone());
            buffers.headers.insert(id, headers);
        }
    }
}

#[no_mangle]
pub extern "C" fn init()  {
    let formatter : Formatter3164 = Formatter3164 {
        facility: Facility::LOG_USER,
        hostname: None,
        process: "analyzer".to_string(),
        pid: 0,
    };

    let logger : Logger::<LoggerBackend, Formatter3164> = match syslog::unix(formatter) {
        Err(e) => { println!("impossible to connect to syslog: {:?}", e); None },
        Ok(_logger) => Some(_logger),
    }.unwrap();

    match log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
        .map(|()| log::set_max_level(LevelFilter::Info)) {
        Err(e) => {
            info!("Logger initialization errored with: {}", e);
        },
        _ => {
            info!("Logger initialized");
        }
    };

    setup_hooks();
}

#[no_mangle]
pub extern "C" fn done(id: i64) {
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => { buffer.done(); },
        None => ()
    }
}
