#![feature(panic_info_message)]
#![feature(deadline_api)]

use flate2::read::GzDecoder;
use log::{LevelFilter, info};
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
use zstream::Decoder;

static mut BUFFERS: Option<Buffers> = None;
static ONCE_BUFFERS: Once = Once::new();

const WINDOW_LENGTH : usize = 128;
const BUFF_SIZE: usize = 128;

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
    //writer: flate2::write::GzEncoder<OutputWriter>,
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
    decoder: Decoder,
    decoder_sender: Sender<Vec<u8>>,
    error: bool,
}

#[repr(C)]
pub struct Chunk {
    size: usize,
    bytes: *const c_void
}


fn append_file(id: i64, prefix: String, data: &[u8]) {
    let filename = format!("{}-{}.log", prefix, id);
    let mut f = OpenOptions::new().create(true).write(true).append(true).open(filename).unwrap();
    f.write_all(data);
}

fn update_file(id: i64, prefix: String, data: &[u8]) {
    let filename = format!("{}-{}.log", prefix, id);
    let mut f = OpenOptions::new().create(true).write(true).append(false).open(filename).unwrap();
    f.write_all(data);
}

fn consume(buf: &mut [u8], receiver: &mut std::sync::mpsc::Receiver<Vec<u8>>) -> Result<usize, String> {
    let max_bytes = buf.len();
    let mut received = Vec::<u8>::new();

    info!("================================= Consume start =================================");

    for slice in receiver.try_iter() {
        if slice.len() + received.len() > buf.len() {
            info!("Not enough bytes on destination buffer");
            return Err(format!("Not enough bytes in buffer to receive: {} vs. {}", slice.len() + received.len(), buf.len()));
        }

        info!("Consumed {} bytes from receiver.", slice.len());

        received.extend(slice);
    }

    info!("================================= Consume end =================================");
    buf[0..received.len()].copy_from_slice(received.as_slice());
    info!("Done consuming {} total bytes from receiver.", received.len());
    Ok(received.len())
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
        
        let (input_sender, input_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();
        let (output_sender, output_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>)= mpsc::channel();

        let output_writer = OutputWriter { name: "Output".to_string(), sender: output_sender };
        let gz_encoder = flate2::write::GzEncoder::new(output_writer, flate2::Compression::default());
        let transformation_writer = BufferWriter { name: "Transformation".to_string(), writer: Box::new(gz_encoder), inject: true };
        let gz_decoder = BufferWriter { name: "Input".to_string(), writer: Box::new(flate2::write::GzDecoder::new(transformation_writer)), inject: false };

        let (decoder_sender, decoder_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

        info!("Initializing buffer {}", id);

        let b = Buffer {
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
            decoder: Decoder::new_with_size(
                BufferReader { id: id, state: 0, name: "input reader".to_string(), receiver: decoder_receiver, consumed: 0, pending: Vec::<u8>::new() },
                BUFF_SIZE,
            ),
            decoder_sender: decoder_sender,
            error: false,
        };

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

    fn read_encoded(&mut self, decoded_len: usize, decoded: &mut [u8], encoded: &mut [u8] ) -> std::io::Result<usize> {
        match &self.encoding{
            Some(encoding) => {
                info!("Sending down {} bytes for encoder reader.", decoded_len);
                match write_bytes_windowed(self.is_done, &decoded[0..decoded_len], WINDOW_LENGTH, &mut self.senders[3], &mut self.pending_encode){
                    Ok(written_bytes) => {
                        if written_bytes > 0 {
                            self.encoding_available = true;
                        } else {
                            self.encoding_available = false;
                        }
                        info!("Sent {} / {} bytes  to encoder reader, {} left pending.", written_bytes, decoded_len, self.pending_encode.len());
                    },
                    Err(SendError(v)) => {
                        info!("Error in read_encoded.");
                    }
                };
                //self.senders[3].send(decoded[0..decoded_len].to_vec());
                if self.encoding_available == true {
                    info!("Will now read from gz encoder.");
                    match self.gz_encoder().read(encoded) {
                        Ok(bytes) => {
                            info!("Done reading {} bytes from gz encoder.", bytes);
                            Ok(bytes)
                        },
                        Err(msg) => {
                            info!("Failed reading from gz encoder: {}", msg);
                            Err(msg)
                        }
                    }
                } else {
                    Ok(0)
                }
            },
            _ => { 
                info!("{} -> {}", decoded.len(), encoded.len());
                encoded.copy_from_slice(decoded);
                Ok(decoded_len)
            }
        }
    }

    fn write_bytes(&mut self, data: &[u8]) {
        match self.decoder_sender.send(data.to_vec()) {
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

    fn write_bytes_old(&mut self, data: &[u8]) {
        info!("Write bytes got {} bytes to store.", data.len());
        let bytes = data.len();
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Will write to gzip decoder buffer");
                    match write_bytes_windowed(self.is_done, data, WINDOW_LENGTH, &mut self.senders[1], &mut self.pending_decode) {
                        Ok(written_bytes) => {
                            if written_bytes > 0 {
                                self.decoding_available = true;
                            } else {
                                self.decoding_available = false;
                            }
                            info!("Sent {}/{} bytes into decoder reader, {} bytes left pending", written_bytes, bytes, self.pending_decode.len()); 
                        },
                        Err(SendError(v)) => { info!("Failed to write into gz decoder buff."); },
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

fn write_bytes_windowed(is_done: bool, data: &[u8], window_size: usize, sender: &mut std::sync::mpsc::Sender<Vec<u8>>, pending: &mut Vec<u8>) -> std::result::Result<usize, SendError<Vec<u8>>> {
    let mut written = 0;

    let mut buffer = Vec::<u8>::new();
    buffer.extend(pending.to_vec());
    buffer.extend(data);
    pending.clear();

    while written < buffer.len() {
        info!("Windowed write: {} / {}", written, pending.len());
        let to_write = min(buffer.len(), window_size);
        let to_write_window = min(buffer.len(), written + to_write);
        let window_slice = buffer[written..to_write_window].to_vec();

        if !is_done && window_slice.len() < window_size {
            info!("Cannod send {} bytes, as window size is {}. {} total bytes pending.", window_slice.len(), window_size, pending.len());
            pending.extend(window_slice);
            break;
        } else {
            sender.send(window_slice)?;
            written = to_write_window;
        }
    }

    info!("Windowed write done: {} / {}", written, data.len());
    std::result::Result::Ok(written)
}

impl Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &self.encoding {
            Some(encoding) => {
                if encoding == "gzip" {
                    info!("Decoding with gzip.");
                    if self.decoding_available {
                        self.gz_decoder().read(buf)
                    } else {
                        Ok(0)
                    }
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

    /*let filename = format!("/tmp/processed-request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    file.expect("Unable to open file.").write_all(&processed).ok();

    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Finished processing. Content encoding is {}.\n", encoding);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();*/
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

            if buffer.error {
                return Chunk {
                    size:0, bytes: null(),
                };
            }

            /*if buffer.bytes_total < MIN {
                return Chunk {
                    size: 0, bytes: null()
                };
            }*/

            let mut output_buffer : [u8; 32 * 1024 ] = [0; 32 * 1024];

            let bytes = match buffer.decoder.read(&mut output_buffer) {
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

            update_file(buffer.id, "decoder-in".to_string(), buffer.decoder.bytes_in());
            update_file(buffer.id, "decoder-out".to_string(), buffer.decoder.bytes_out());

            buffer.transfer_chunk = output_buffer[0..bytes].to_vec();
            transform(buffer.transfer_chunk.as_mut_slice())
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
    /*let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Got a chunk with size {}\n", size);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();*/
}

#[no_mangle]
pub extern "C" fn commit(id: i64, content_encoding: *const c_char, uri: *const c_char) {
    /*let encoding = unsafe {CStr::from_ptr(content_encoding)}.to_str().unwrap().to_owned();
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = format!("Finished appending, content transfer complete. Content encoding is {}.\n", encoding);
    file.expect("Unable to open file.").write_all(content.as_bytes()).ok();

    let buffers = get_buffers();
    let mut buffer_ref = buffers.responses.remove(&id);*/

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

#[no_mangle]
pub extern "C" fn content_done(id: i64) {
    let buffers = get_buffers();
    match buffers.responses.get_mut(&id) {
        Some(buffer) => { buffer.done(); },
        None => ()
    }
}
