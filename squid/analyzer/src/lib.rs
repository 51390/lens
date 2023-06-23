#![feature(panic_info_message)]

use flate2::read::GzDecoder;
use log::{LevelFilter, info};
use std::collections::HashMap;
use std::cmp::min;
use std::ffi::{
    c_void,
    c_char,
    CStr
};
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::ptr::null;
use std::sync::Once;
use std::thread;
use std::vec::Vec;
use syslog::{Logger, LoggerBackend, Facility, Formatter3164, BasicLogger};
use std::sync::mpsc::{Sender, Receiver, TryRecvError, SendError};
use std::sync::mpsc;


static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

trait Instance<T> {
    fn new() -> Option<T>;
}

struct BufferReader {
    consumed: usize,
    receiver: Receiver<Vec<u8>>,
    name: String,
    state: u32,
}

struct Buffer {
    id: i64,
    consumed: usize,
    encoding: Option<String>,
    transfer_chunk: Vec<u8>,
    gz_decoder: Option<flate2::read::GzDecoder<BufferReader>>,
    gz_encoder: Option<flate2::read::GzEncoder<BufferReader>>,
    br_decoder: brotli_decompressor::Decompressor<BufferReader>,
    reader: BufferReader,
    senders: [Sender<Vec<u8>>; 4],
    available_receivers: [Option<Receiver<Vec<u8>>>; 4],

}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}

fn consume(buf: &mut [u8], receiver: &mut std::sync::mpsc::Receiver<Vec<u8>>) -> Result<usize, String> {
    let max_bytes = buf.len();
    let mut received = Vec::<u8>::new();

    for slice in receiver.try_iter() {
        if slice.len() + received.len() > buf.len() {
            return Err(format!("Not enough bytes in buffer to receive: {} vs. {}", slice.len() + received.len(), buf.len()));
        }

        info!("Consumed {} bytes from receiver.", slice.len());

        received.extend(slice);
    }

    buf[0..received.len()].copy_from_slice(received.as_slice());
    info!("Done consuming {} total bytes from receiver.", received.len());
    Ok(received.len())
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        info!("BufferReader.read called for {}", self.name);

        match consume(buf, &mut self.receiver) {
            Ok(bytes) => {
                info!("BF({}) -> Got {} bytes from channel.", self.name, bytes);
                Ok(bytes)
            },
            Err(msg) => {
                info!("BF({}) -> Failed getting data from channel, returning 0 bytes read. Error: {}", self.name, msg);
                Ok(0)
            }
        }

        /*
        let mut end = self.bytes.len();
        let start = min(self.consumed, end);
        if end - start > buf.len() {
            end = start + buf.len();
        }
        buf[0..end-start].clone_from_slice(&self.bytes[start..end]);
        self.consumed = end;
        info!("BufferReader passing on {} bytes, buf len is now {}", end - start, buf.len());
        return Ok(end - start);
        */
    }
}

/*

impl BufferReader {
    fn buff_write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend(buf);
        info!("BufferReader wrote {} bytes, total inner buffer is now at {}", buf.len(), self.bytes.len());
        return Ok(buf.len());
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
*/

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

        info!("Initializing buffer {}", id);

        let b = Buffer {
            id: id,
            encoding: encoding.cloned(),
            consumed: 0,
            transfer_chunk: Vec::<u8>::new(),
            reader: BufferReader{ state: 0, name: "raw".to_string(), receiver: receiver1, consumed: 0 },
            gz_decoder: None,
            gz_encoder: None,
            br_decoder: brotli_decompressor::Decompressor::new(BufferReader{ state: 0, name: "br".to_string(), receiver: receiver3, consumed: 0 }, 8192),
            senders: [sender1, sender2, sender3, sender4 ],
            available_receivers: [ None, Some(receiver2), None, Some(receiver4)], // receivers left to initialize yet
        };


        info!("Done initializing buffer {}", id);

        b
    }

    fn gz_decoder(&mut self) -> &mut flate2::read::GzDecoder<BufferReader> {
        match &self.gz_decoder {
            None => {
                self.gz_decoder =
                    Some(flate2::read::GzDecoder::new(BufferReader{ state: 0, name: "gz decoder".to_string(), receiver: self.available_receivers[1].take().unwrap(), consumed: 0 }));
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
                        BufferReader{ state: 0, name: "gz encoder".to_string(), receiver: self.available_receivers[3].take().unwrap(), consumed: 0 },
                        flate2::Compression::new(9)
                    ));
            },
            _ => (),
        };

        self.gz_encoder.as_mut().unwrap()
    }

    fn read_encoded(&mut self, decoded_len: usize, decoded: &mut [u8], encoded: &mut [u8] ) -> std::io::Result<usize> {
        match &self.encoding{
            Some(encoding) => {
                info!("Sending down {} bytes for encoder reader.", decoded_len);
                write_bytes_windowed(&decoded[0..decoded_len], 64, &mut self.senders[3]);
                //self.senders[3].send(decoded[0..decoded_len].to_vec());
                self.gz_encoder().read(encoded)
            },
            _ => { 
                info!("{} -> {}", decoded.len(), encoded.len());
                encoded.copy_from_slice(decoded);
                Ok(decoded_len)
            }
        }
    }

    fn write_bytes(&mut self, data: &[u8]) {
        info!("Write bytes got {} bytes to store.", data.len());
        let bytes = data.len();
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Will write to gzip decoder buffer");
                    match write_bytes_windowed(data, 32768, &mut self.senders[1]) {
                        Err(SendError(v)) => { info!("Failed to write into gz decoder buff."); },
                        _ => { info!("Wrote {} bytes into gz decoder buff", bytes); },
                    };
                } else if encoding == "br" {
                    info!("Using br decoder");
                    match self.senders[0].send(data.to_vec()) {
                        Err(SendError(_)) => { info!("Failed to write into gz decoder buff."); },
                        _ => { info!("Wrote {} bytes into gz decoder buff", bytes); },
                    };
               } else {
                    info!("Using no decoder");
                    match self.senders[0].send(data.to_vec()) {
                        Err(SendError(_)) => { info!("Failed to write into gz decoder buff."); },
                        _ => { info!("Wrote {} bytes into gz decoder buff", bytes); },
                    };
                }
            },
            _ => { 
                info!("Using no decoder");
                match self.senders[0].send(data.to_vec()) {
                    Err(SendError(_)) => { info!("Failed to write into gz decoder buff."); },
                    _ => { info!("Wrote {} bytes into gz decoder buff", bytes); },
                };
            },
        }
    }
}

fn write_bytes_windowed(data: &[u8], window_size: usize, sender: &mut std::sync::mpsc::Sender<Vec::<u8>>) -> std::result::Result<(), SendError<Vec::<u8>>> {
    let mut written = 0;

    while written < data.len() {
        info!("Windowed write: {} / {}", written, data.len());
        let to_write = min(data.len(), window_size);
        let to_write_window = min(data.len(), written + to_write);
        let window_slice = &data[written..to_write_window];
        sender.send(window_slice.to_vec())?;
        written += to_write;
    }

    info!("Windowed write done: {} / {}", written, data.len());
    std::result::Result::Ok(())
}

impl Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Decoding with gzip.");
                    self.gz_decoder().read(buf)
                    //self.reader.read(buf)
                } /*else if encoding == "br" {
                    info!("Decoding with brotli.");
                    self.br_decoder.read(buf)
                } */else {
                    info!("Unknown encoding, will send raw: {}", encoding);
                    self.reader.read(buf)
                }
            },
            None => {
                info!("No encoding, will send raw.");
                self.reader.read(buf)
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

    let filename = format!("/tmp/processed-request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    file.expect("Unable to open file.").write_all(&processed).ok();

    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Finished processing. Content encoding is {}.\n", encoding);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();
}

fn transform(content: &mut [u8] ) -> Chunk {
    info!("Transfrn retyrubg chunk of size {}.", content.len());
    Chunk {
        size: content.len(),
        bytes: content.as_ptr() as *const c_void,
    }
}

#[no_mangle]
pub extern "C" fn get_content(id: i64) -> Chunk {
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => {
            //let consumed = buffer.get_bytes().len();
            //let content = &mut buffer.get_bytes()[buffer.consumed..consumed];
            //buffer.consumed = consumed;
            let mut content = [0u8; 81920];

            let result = buffer.read(&mut content);


            let filename = format!("/tmp/content-transfer-{}.log", id);
            let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
            match result {
                Ok(bytes) => {
                    let mut encoded = [0u8; 81920];
                    let mut encoder = buffer.gz_encoder();

                    info!("Will encode data now: {} bytes.", bytes);

                    match buffer.read_encoded(bytes, &mut content, &mut encoded) {
                        Ok(encoded_bytes) => {
                            info!("Get content called for id {}; {} bytes on buffer", id, bytes);
                            file.expect("Unable to open file.").write_all(&encoded[0..encoded_bytes]).ok();
                            buffer.transfer_chunk = encoded[0..encoded_bytes].to_vec();

                            transform(buffer.transfer_chunk.as_mut_slice())
                        },
                        Err(message) => {
                            info!("Failed re-encoding with {}.", message);

                            Chunk { size: 0, bytes: null() }
                        }
                    }

                },
                Err(message) => {
                    let msg = format!("Error reading content: {}\n", message);
                    info!("Get content failed for id {} with {}", id, msg);
                    file.expect("Unable to open file.").write_all(msg.as_bytes()).ok();

                    Chunk { size: 0, bytes: null() }
                },
            }
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
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Got a chunk with size {}\n", size);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();
}

#[no_mangle]
pub extern "C" fn commit(id: i64, content_encoding: *const c_char, uri: *const c_char) {
    let encoding = unsafe {CStr::from_ptr(content_encoding)}.to_str().unwrap().to_owned();
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Finished appending, content transfer complete. Content encoding is {}.\n", encoding);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();

    let buffers = get_buffers();
    let mut buffer_ref = buffers.responses.remove(&id);

    /*
    match buffer_ref {
        Some(mut buffer) => {
            let bytes = buffer.get_bytes().to_vec();
            thread::spawn(move || {process(&id, &bytes, &encoding)});
        },
        _ => ()
    };
    */
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

    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("HEADER {} -> {} (uri: {})\n", name, value, uri);
    info!("{}", content);
    //file.expect("Unable to open file.").write_all(content.as_bytes()).ok();
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
