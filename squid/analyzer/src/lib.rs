use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::ffi::{
    c_void,
    c_char,
    CStr
};
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::sync::Once;
use std::thread;
use std::vec::Vec;

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

trait Instance {
    fn new() -> Option<Buffers>;
}

struct Buffers {
    transactions: HashMap<i64, Vec<u8>>,
}

impl Instance for Buffers {
    fn new() -> Option<Buffers> {
        Some(Buffers { transactions: HashMap::new() })
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
    match buffers.transactions.get_mut(&id) {
        Some(buffer) => unsafe {
            buffer_size = buffer.len();
            buffer.extend(std::slice::from_raw_parts(ptr, size));
        },
        None => unsafe {
            let mut buffer = Vec::<u8>::new();
            buffer_size = buffer.len();
            buffer.extend(std::slice::from_raw_parts(ptr, size));
            drop(buffers.transactions.insert(id, buffer));
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

#[no_mangle]
pub extern "C" fn transfer(id: i64, chunk: *const c_void, size: usize) {
    let buffer_size = append(id, chunk, size);
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Got a chunk with size {}. Buffers has a total of {} bytes.\n", size, buffer_size);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();
}

#[no_mangle]
pub extern "C" fn commit(id: i64, content_encoding: *const c_char) {
    let encoding = unsafe {CStr::from_ptr(content_encoding)}.to_str().unwrap().to_owned();
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Finished appending, content transfer complete. Content encoding is {}.\n", encoding);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();

    let buffers = get_buffers();
    let buffer_ref = buffers.transactions.remove(&id);
    match buffer_ref {
        Some(buffer) => {
            thread::spawn(move || {process(&id, &buffer, &encoding)});
        },
        _ => ()
    };

}
