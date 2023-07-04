#![feature(panic_info_message)]
#![feature(deadline_api)]

use flate2::read::GzDecoder;
use log::{LevelFilter, info, warn};
use std::boxed::Box;
use std::collections::HashMap;
use std::cmp::min;
use std::ffi::{
    c_void,
    c_char,
    CStr
};
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::io::{Error, ErrorKind};
use std::ptr::null;
use std::sync::Once;
use std::thread;
use std::vec::Vec;
use syslog::{Logger, LoggerBackend, Facility, Formatter3164, BasicLogger};
use std::sync::mpsc::{Sender, Receiver, TryRecvError, SendError, RecvTimeoutError};
use std::sync::mpsc;
use zstream::{Encoder, Decoder};

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

const INPUT_BUFFER_SIZE: usize = 1024 * 1024;
const ENCODER_BUFFER_SIZE : usize =  1024 * 1024;
const OUTPUT_BUFFER_SIZE: usize = 1024 * 1024;

trait Instance<T> {
    fn new() -> Option<T>;
}

struct BufferReader {
    consumed: usize,
    receiver: Receiver<Vec<u8>>,
    name: String,
    state: u32,
    pending: Vec<u8>,
    id: i64,
}

struct BufferWriter {
    name: String,
    writer: Box<dyn Write>,
    inject: bool,
}

struct OutputWriter {
    name: String,
    sender: Sender<Vec<u8>>,
}

impl Write for BufferWriter {

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        info!("({}) writer will send down {} bytes.", self.name, buf.len());

        if self.inject {
            self.inject = false;
            self.writer.write_all("<!--injection here-->".as_bytes()).unwrap();
        }

        match self.writer.write(buf) {
            Ok(bytes) => {
                info!("({}) Ok for {} bytes", self.name, bytes);
                Ok(bytes)
            },
            Err(msg) => {
                info!("({}) failed with {}", self.name, msg);
                Err(msg)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

impl Write for OutputWriter {

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.sender.send(buf.to_vec()) {
            Ok(()) => Ok(buf.len()),
            Err(SendError(v)) => Err(Error::new(ErrorKind::Other, format!("{} writer failed sending {} bytes.", self.name, v.len())))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Buffer {
    id: i64,
    is_done: bool,
    consumed: usize,
    encoding: Option<String>,
    transfer_chunk: Vec<u8>,
    gz_decoder: Option<flate2::read::GzDecoder<BufferReader>>,
    decoding_available: bool,
    gz_encoder: Option<flate2::read::GzEncoder<BufferReader>>,
    encoding_available: bool,
    br_decoder: brotli_decompressor::Decompressor<BufferReader>,
    reader: BufferReader,
    senders: [Sender<Vec<u8>>; 4],
    available_receivers: [Option<Receiver<Vec<u8>>>; 4],
    pending_encode: Vec<u8>,
    pending_decode: Vec<u8>,

    bytes_available: bool,
    bytes_sender: Sender<Vec<u8>>,
    bytes_receiver: Receiver<Vec<u8>>,
    output_receiver: Receiver<Vec<u8>>,
    input_writer: BufferWriter,
    bytes_total: usize,
    //decoder: Decoder,
    encoder: Encoder,
    decoder_sender: Sender<Vec<u8>>,
    error: bool,
}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}


fn append_file(id: i64, prefix: String, data: &[u8]) {
    /*
    let filename = format!("{}-{}.log", prefix, id);
    let mut f = OpenOptions::new().create(true).write(true).append(true).open(filename).unwrap();
    f.write_all(data);
    */
}

fn update_file(id: i64, prefix: String, data: &[u8]) {
    /*
    let filename = format!("{}-{}.log", prefix, id);
    let mut f = OpenOptions::new().create(true).write(true).append(false).open(filename).unwrap();
    f.write_all(data);
    */
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let pending = self.pending.len();
        let n_data = match self.receiver.recv_deadline(std::time::Instant::now() + std::time::Duration::from_millis(200)) {
            Ok(data) => {
                append_file(self.id, "bytes-read".to_string(), &data);
                let n_data = data.len();
                self.pending.extend(data);
                n_data
            },
            RecvTimeoutError => {
                0
            }
            /*
            Err(msg) => {
                info!("BF({}) -> Failed getting data from channel, returning 0 bytes read. Error: {}", self.name, msg);
                Ok(0)
            }*/
        };

        let acum = self.pending.len();
        let mut to_transfer = min(buf.len(), self.pending.len());
        let drained : Vec<u8> = self.pending.drain(0..to_transfer).collect();
        buf[0..to_transfer].copy_from_slice(&drained[0..to_transfer]);

        append_file(self.id, "bytes-transferred".to_string(), &buf[0..to_transfer]);

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

        let (sender1, receiver1) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();
        let (sender2, receiver2) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();
        let (sender3, receiver3) : (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

        let (sender4, receiver4) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();
        
        let (input_sender, input_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();
        let (output_sender, output_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();

        let output_writer = OutputWriter { name: "Output".to_string(), sender: output_sender };
        let gz_encoder = flate2::write::GzEncoder::new(output_writer, flate2::Compression::default());
        let transformation_writer = BufferWriter { name: "Transformation".to_string(), writer: Box::new(gz_encoder), inject: true };
        let gz_decoder = BufferWriter { name: "Input".to_string(), writer: Box::new(flate2::write::GzDecoder::new(transformation_writer)), inject: false };

        let (decoder_sender, decoder_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

        info!("Initializing buffer {}", id);

        let mut b = Buffer {
            id: id,
            is_done: false,
            encoding: encoding.cloned(),
            consumed: 0,
            transfer_chunk: Vec::<u8>::new(),
            reader: BufferReader{ id: id, state: 0, name: "raw".to_string(), receiver: receiver1, consumed: 0, pending: Vec::<u8>::new() },
            gz_decoder: None,
            gz_encoder: None,
            br_decoder: brotli_decompressor::Decompressor::new(BufferReader{ id:id,  state: 0, name: "br".to_string(), receiver: receiver3, consumed: 0, pending: Vec::<u8>::new() }, 8192),
            senders: [sender1, sender2, sender3, sender4 ],
            available_receivers: [ None, Some(receiver2), None, Some(receiver4)], // receivers left to initialize yet
            pending_encode: Vec::<u8>::new(),
            pending_decode: Vec::<u8>::new(),
            decoding_available: false,
            encoding_available: false,
            bytes_sender: input_sender,
            bytes_receiver: input_receiver,
            bytes_available: false,
            input_writer: gz_decoder,
            output_receiver: output_receiver,
            bytes_total: 0,
            encoder: Encoder::new_with_size(
                Decoder::new_with_size(
                    BufferReader { id: id, state: 0, name: "input reader".to_string(), receiver: decoder_receiver, consumed: 0, pending: Vec::<u8>::new() },
                    INPUT_BUFFER_SIZE
                ),
                ENCODER_BUFFER_SIZE
            ),
            decoder_sender: decoder_sender,
            error: false,
        };

        /*
        for i in 0..OUTPUT_BUFFER_SIZE {
            b.transfer_chunk.push(0);
        }
        */

        info!("Done initializing buffer {}", id);

        b
    }

    fn done(&mut self) {
        self.is_done = true;
        self.input_writer.flush().unwrap();
        info!("Transaction {} is set as done.", self.id);
    }

    fn gz_decoder(&mut self) -> &mut flate2::read::GzDecoder<BufferReader> {
        match &self.gz_decoder {
            None => {
                self.gz_decoder =
                    Some(flate2::read::GzDecoder::new(BufferReader{ id: self.id, state: 0, name: "gz decoder".to_string(), receiver: self.available_receivers[1].take().unwrap(), consumed: 0, pending: Vec::<u8>::new() }));
            },
            _ => (),
        };

        self.gz_decoder.as_mut().unwrap()
    }

    fn gz_encoder(&mut self) -> &mut flate2::read::GzEncoder<BufferReader> {
        match &self.gz_encoder {
            None => {
                info!("Initializing GZ Encoder.");
                self.gz_encoder =
                    Some(flate2::read::GzEncoder::new(
                        BufferReader{ id: self.id, state: 0, name: "gz encoder".to_string(), receiver: self.available_receivers[3].take().unwrap(), consumed: 0, pending: Vec::<u8>::new() },
                        flate2::Compression::new(9)
                    ));
            },
            _ => (),
        };

        self.gz_encoder.as_mut().unwrap()
    }

    fn write_bytes(&mut self, data: &[u8]) {
        let mut sender = {
            match &self.encoding {
                Some(encoding) => {
                    //&self.bytes_sender
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
                append_file(self.id, "bytes-written".to_string(), data);
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

fn brotli_decompress(buffer: &[u8]) -> Vec<u8> {
    let mut decompressor = brotli_decompressor::Decompressor::new(buffer, buffer.len());
    let mut decoded = Vec::new();
    decompressor.read_to_end(&mut decoded);
    decoded
}

fn gzip_decompress(buffer: &[u8]) -> Vec<u8> {
    let mut decoder = GzDecoder::new(buffer);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded);
    decoded
}

fn process(id: &i64, buffer: &[u8], encoding: &str) {
    let processed = match encoding {
        "gzip" => gzip_decompress(buffer),
        //"br" => brotli_decompress(buffer),
        _ => buffer.to_vec(),
    };
}

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
                           TryRecvError => {
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
                        TryRecvError => {
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
                    info!("Read {} bytes from decoder.", bytes);
                    bytes
                },
                Err(e) => {
                    info!("Failed reading will return 0 bytes. Error {}", e);
                    buffer.error = true;
                    0
                }
            };

            update_file(buffer.id, "encoder-in".to_string(), buffer.encoder.bytes_in());
            update_file(buffer.id, "encoder-out".to_string(), buffer.encoder.bytes_out());

            buffer.transfer_chunk = output_buffer[0..bytes].to_vec();
            transform(bytes, buffer.transfer_chunk.as_mut_slice())
        },
        None => {
            info!("Get content called for id {}; buffer has not been initialized yet.", id);
            Chunk { size: 0, bytes: null() }
        }
    }
}

#[no_mangle]
pub extern "C" fn transfer(id: i64, chunk: *const c_void, size: usize, uri: *const c_char) {
    append(id, chunk, size);
}

#[no_mangle]
pub extern "C" fn commit(id: i64, content_encoding: *const c_char, uri: *const c_char) {
}

#[no_mangle]
pub extern "C" fn header(id: i64, name: *const c_char, value: *const c_char, uri: *const c_char) {
    let uri = unsafe {CStr::from_ptr(uri)}.to_str().unwrap().to_owned();
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
        .map(|()| log::set_max_level(LevelFilter::Info));

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
