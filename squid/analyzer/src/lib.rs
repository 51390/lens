#![feature(panic_info_message)]
#![feature(deadline_api)]

use log::{LevelFilter, info, warn};
use std::boxed::Box;
use std::collections::HashMap;
use std::cmp::min;
use std::ffi::{
    c_void,
    c_char,
    CStr
};
use std::io::prelude::*;
use std::ptr::null;
use std::sync::Once;
use std::vec::Vec;
use syslog::{Logger, LoggerBackend, Facility, Formatter3164, BasicLogger};
use std::sync::mpsc::{channel, Sender, Receiver, SendError};
use zstream::{Encoder, Decoder};

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

const INPUT_BUFFER_SIZE: usize = 2 * 1024 * 1024;
const ENCODER_BUFFER_SIZE : usize =  2 * 1024 * 1024;
const OUTPUT_BUFFER_SIZE: usize = 2 * 1024 * 1024;

trait Instance<T> {
    fn new() -> Option<T>;
}

struct BufferReader {
    receiver: Receiver<Vec<u8>>,
    name: String,
    pending: Vec<u8>,
}

struct Buffer {
    id: i64,
    is_done: bool,
    encoding: Option<String>,
    transfer_chunk: Vec<u8>,
    bytes_total: usize,
    bytes_sender: Sender<Vec<u8>>,
    bytes_receiver: Receiver<Vec<u8>>,
    encoder: Encoder,
    decoder_sender: Sender<Vec<u8>>,
    error: bool,
}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let pending = self.pending.len();
        let n_data = match self.receiver.recv_deadline(std::time::Instant::now() + std::time::Duration::from_millis(200)) {
            Ok(data) => {
                let n_data = data.len();
                self.pending.extend(data);
                n_data
            },
            Err(_) => {
                0
            }
        };

        let acum = self.pending.len();
        let to_transfer = min(buf.len(), self.pending.len());
        let drained : Vec<u8> = self.pending.drain(0..to_transfer).collect();
        buf[0..to_transfer].copy_from_slice(&drained[0..to_transfer]);

        info!(
            "BF({}) -> {} pending; {} in; {} acum; {} transfer; {} drained; {} left.",
            self.name, pending, n_data, acum, to_transfer, drained.len(), self.pending.len()
        );

        Ok(to_transfer)
    }
}

struct Buffers {
    responses: HashMap<i64, Buffer>,
    headers: HashMap<i64, HashMap<String, String>>,
}

impl Buffer {
    fn new(id: i64) -> Buffer {
        let buffers = get_buffers();
        let encoding = match buffers.headers.get(&id) {
            Some(headers) => {
                headers.get("Content-Encoding")
            },
            _ => None
        };

        let (bytes_sender, bytes_receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
        let (decoder_sender, decoder_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();

        Buffer {
            id: id,
            is_done: false,
            encoding: encoding.cloned(),
            transfer_chunk: Vec::<u8>::new(),
            bytes_total: 0,
            bytes_sender: bytes_sender,
            bytes_receiver: bytes_receiver,
            encoder: Encoder::new_with_size(
                Decoder::new_with_size(
                    BufferReader { name: "input reader".to_string(), receiver: decoder_receiver, pending: Vec::<u8>::new() },
                    INPUT_BUFFER_SIZE
                ),
                ENCODER_BUFFER_SIZE
            ),
            decoder_sender: decoder_sender,
            error: false,
        }
    }

    fn done(&mut self) {
        self.is_done = true;
        info!("Transaction {} is set as done.", self.id);
    }

    fn write_bytes(&mut self, data: &[u8]) {
        let sender = {
            match &self.encoding {
                Some(encoding) => {
                    if encoding == "gzip" {
                        &self.decoder_sender
                    } else {
                        &self.bytes_sender
                    }
                },
                None => &self.bytes_sender
            }
        };

        match sender.send(data.to_vec()) {
            Ok(()) => {
                self.bytes_total += data.len();
                info!("Sent {} bytes down to processing.", data.len());
            },
            Err(SendError(sent)) => {
                info!("Failed to send {} bytes", sent.len());
            }
        }
    }
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
        None => unsafe {
            let mut buffer = Buffer::new(id);
            buffer.write_bytes(std::slice::from_raw_parts(ptr, size));
            drop(buffers.responses.insert(id, buffer));
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
    info!("Transfrn retyrubg chunk of size {}.", content.len());
    Chunk {
        size: bytes,
        bytes: content.as_ptr() as *const c_void,
    }
    //Chunk { size: 0, bytes: null(), }
}

#[no_mangle]
pub extern "C" fn get_content(id: i64) -> Chunk {
    const MIN : usize = 1024;
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => {
            //let mut output_buffer = buffer.transfer_chunk.as_mut_slice();
            let mut output_buffer : [u8; OUTPUT_BUFFER_SIZE ] = [0; OUTPUT_BUFFER_SIZE];
            if !buffer.is_done && buffer.bytes_total < MIN {
                return Chunk { size: 0, bytes: null() };
            }

            match &buffer.encoding {
                Some(encoding) => {
                    if encoding != "gzip" {
                       match buffer.bytes_receiver.try_recv() {
                           Ok(bytes) => {
                               buffer.transfer_chunk = bytes;
                               return transform(buffer.transfer_chunk.len(), &mut buffer.transfer_chunk);
                           },
                           Err(_) => {
                               warn!("Failed reading non-gzip stream");
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
                            warn!("Failed reading non-gzip stream");
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

            let result = {
                if buffer.is_done {
                    buffer.encoder.finish(&mut output_buffer)
                } else {
                    buffer.encoder.read(&mut output_buffer)
                }
            };

            let bytes = match  result {
                Ok(bytes) => {
                    //info!("Read {} bytes from decoder.", bytes);
                    bytes
                },
                Err(e) => {
                    info!("Failed reading will return 0 bytes. Error {}", e);
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
pub extern "C" fn transfer(id: i64, chunk: *const c_void, size: usize, _uri: *const c_char) {
    append(id, chunk, size);
}

#[no_mangle]
pub extern "C" fn commit(_id: i64, _content_encoding: *const c_char, _uri: *const c_char) {
}

#[no_mangle]
pub extern "C" fn header(id: i64, name: *const c_char, value: *const c_char, _uri: *const c_char) {
    //let uri = unsafe {CStr::from_ptr(uri)}.to_str().unwrap().to_owned();
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

fn setup_hooks() {
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if let Some(message) = panic_info.message() {
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

    log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
        .map(|()| log::set_max_level(LevelFilter::Info)).unwrap();

    setup_hooks();
}

#[no_mangle]
pub extern "C" fn content_done(id: i64) {
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => { buffer.done(); },
        None => ()
    }
}
