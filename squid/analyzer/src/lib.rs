use flate2::read::GzDecoder;
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

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

trait Instance<T> {
    fn new() -> Option<T>;
}

struct BufferReader<'a> {
    consumed: usize,
    bytes: &'a Vec<u8>,
}

struct Buffer {
    id: i64,
    bytes: Vec<u8>,
    consumed: usize,
    encoding: Option<String>,
}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}

impl<'a> Read for BufferReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut end = self.bytes.len();
        let start = min(self.consumed, end);
        if end - start > buf.len() {
            end = start + buf.len();
        }
        buf.clone_from_slice(&self.bytes[start..end]);
        self.consumed = end;
        return Ok(end - start);
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
                headers.get("content-encoding")
            },
            _ => None
        };

        Buffer {
            id: id,
            bytes: Vec::<u8>::new(),
            encoding: encoding.cloned(),
            consumed: 0
        }
    }
}

impl Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut reader : Box<dyn Read> = Box::new(BufferReader {
            bytes: &self.bytes,
            consumed: self.consumed,
        });

        reader = match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    Box::new(GzDecoder::new(reader))
                } else if encoding == "br" {
                    Box::new(brotli_decompressor::Decompressor::new(reader, 8192))
                } else {
                    reader
                }
            },
            None => reader
        };

        reader.read(buf)
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
            buffer_size = buffer.bytes.len();
            buffer.bytes.extend(std::slice::from_raw_parts(ptr, size));
        },
        None => unsafe {
            let mut buffer = Buffer::new(id);
            buffer_size = buffer.bytes.len();
            buffer.bytes.extend(std::slice::from_raw_parts(ptr, size));
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
            let consumed = buffer.bytes.len();
            let content = &mut buffer.bytes[buffer.consumed..consumed];
            buffer.consumed = consumed;
            transform(content)
        },
        None => Chunk { size: 0, bytes: null() }
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
    let buffer_ref = buffers.responses.remove(&id);
    match buffer_ref {
        Some(buffer) => {
            let bytes = buffer.bytes;
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
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();
}
