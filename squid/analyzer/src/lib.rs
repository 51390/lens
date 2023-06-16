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



static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

trait Instance<T> {
    fn new() -> Option<T>;
}

struct BufferReader {
    consumed: usize,
    bytes: Vec<u8>,
}

struct Buffer {
    id: i64,
    consumed: usize,
    encoding: Option<String>,
    transfer_chunk: Vec<u8>,
    gz_decoder: flate2::read::GzDecoder<BufferReader>,
    br_decoder: brotli_decompressor::Decompressor<BufferReader>,
    reader: BufferReader,

}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        info!("BufferReader consumed {} bytes, buf len is now {}", self.consumed, buf.len());
        let mut end = self.bytes.len();
        let start = min(self.consumed, end);
        if end - start > buf.len() {
            end = start + buf.len();
        }
        buf[0..end-start].clone_from_slice(&self.bytes[start..end]);
        self.consumed = end;
        info!("BufferReader passing on {} bytes, buf len is now {}", end - start, buf.len());
        return Ok(end - start);
    }
}

impl Write for BufferReader {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend(buf);
        info!("BufferReader wrote {} bytes, total inner buffer is now at {}", buf.len(), self.bytes.len());
        return Ok(buf.len());
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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

        Buffer {
            id: id,
            encoding: encoding.cloned(),
            consumed: 0,
            transfer_chunk: Vec::<u8>::new(),
            reader: BufferReader{ bytes: Vec::<u8>::new(), consumed: 0 },
            gz_decoder: flate2::read::GzDecoder::new(BufferReader{ bytes: Vec::<u8>::new(), consumed: 0 }),
            br_decoder: brotli_decompressor::Decompressor::new(BufferReader{ bytes: Vec::<u8>::new(), consumed: 0 }, 8192),
        }
    }

    fn get_bytes(&mut self) -> &mut Vec::<u8> {
        &mut self.reader.bytes
    }

    fn write_bytes(&mut self, data: &[u8]) {
        info!("Write bytes got {} bytes to store.", data.len());
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Using gzip decoder");
                    //let br = self.gz_decoder.get_mut();
                    let mut br = &mut self.gz_decoder;
                    match br.write(data) {
                        Ok(bytes) => { info!("Wrote {} bytes into gz decoder buff", bytes); },
                        Err(msg) => { info!("Failed to write into gz decoder buff with {}", msg); }
                    };
                } else if encoding == "br" {
                    info!("Using no decoder");
                    self.reader.write(data);
                } else {
                    info!("Using no decoder");
                    self.reader.write(data);
                }
            },
            _ => { 
                info!("Using no decoder");
                self.reader.write(data); 
            },

        }
    }
}

impl Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Decoding with gzip.");
                    self.gz_decoder.read(buf)
                } else if encoding == "br" {
                    info!("Decoding with brotli.");
                    self.br_decoder.read(buf)
                } else {
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

fn append(id: i64, chunk: *const c_void, size: usize) -> usize {
    let ptr = chunk as *const u8;
    let buffers = get_buffers();
    let buffer_size;
    match buffers.responses.get_mut(&id) {
        Some(buffer) => unsafe {
            buffer_size = buffer.get_bytes().len();
            //buffer.get_bytes().extend(std::slice::from_raw_parts(ptr, size));
            buffer.write_bytes(std::slice::from_raw_parts(ptr, size));
        },
        None => unsafe {
            let mut buffer = Buffer::new(id);
            buffer_size = buffer.get_bytes().len();
            //buffer.get_bytes().extend(std::slice::from_raw_parts(ptr, size));
            buffer.write_bytes(std::slice::from_raw_parts(ptr, size));
            drop(buffers.responses.insert(id, buffer));
        },
    }
    buffer_size
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
        "br" => brotli_decompress(buffer),
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
            let mut content : Vec<u8> = vec![0;  1];

            let result = buffer.read(content.as_mut_slice());


            let filename = format!("/tmp/content-transfer-{}.log", id);
            let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
            match result {
                Ok(bytes) => {
                    info!("Get content called for id {}; {} bytes on buffer", id, bytes);
                    file.expect("Unable to open file.").write_all(&content[0..bytes]).ok();
                    buffer.transfer_chunk = content[0..bytes].to_vec();

                    transform(buffer.transfer_chunk.as_mut_slice())
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
    let buffer_size = append(id, chunk, size);
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Got a chunk with size {}. Buffers has a total of {} bytes.\n", size, buffer_size);
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
    match buffer_ref {
        Some(mut buffer) => {
            let bytes = buffer.get_bytes().to_vec();
            thread::spawn(move || {process(&id, &bytes, &encoding)});
        },
        _ => ()
    };
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
}
