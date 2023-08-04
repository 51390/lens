use zstream::{Decoder,Encoder};
use std::cmp::min;
use std::io::prelude::*;
use std::vec::Vec;
use std::sync::mpsc::{channel, Sender, SendError, Receiver};
use log::{info, error};

const INPUT_BUFFER_SIZE: usize = 32 * 1024;
const ENCODER_BUFFER_SIZE : usize =  1024 * 1024;

struct BufferReader {
    id: i64,
    receiver: Receiver<Vec<u8>>,
    name: String,
    pending: Vec<u8>,
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        //let pending = self.pending.len();
        let _n_data = match self.receiver.try_recv() {
            Ok(data) => {
                let n_data = data.len();
                self.pending.extend(data);
                n_data
            },
            Err(_) => 0,
        };

        //let acum = self.pending.len();
        let to_transfer = min(buf.len(), self.pending.len());
        let drained : Vec<u8> = self.pending.drain(0..to_transfer).collect();
        buf[0..to_transfer].copy_from_slice(&drained[0..to_transfer]);

        /*info!(
            "BF({}) -> {} pending; {} in; {} acum; {} transfer; {} drained; {} left.",
            self.name, pending, n_data, acum, to_transfer, drained.len(), self.pending.len()
        );*/

        Ok(to_transfer)
    }
}

pub struct Buffer {
    pub id: i64,
    pub uri: String,
    pub is_done: bool,
    pub encoding: Option<String>,
    pub transfer_chunk: Vec<u8>,
    pub bytes_total: usize,
    pub bytes_sender: Sender<Vec<u8>>,
    pub bytes_receiver: Receiver<Vec<u8>>,
    pub encoder: Encoder,
    pub decoder_sender: Sender<Vec<u8>>,
    pub error: bool,
}

impl Buffer {
    pub fn new(id: i64, uri: String, encoding: Option<&String>) -> Buffer {
        let (bytes_sender, bytes_receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
        let (decoder_sender, decoder_receiver) : (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();

        Buffer {
            id: id,
            uri: uri,
            is_done: false,
            encoding: encoding.cloned(),
            transfer_chunk: Vec::<u8>::new(),
            bytes_total: 0,
            bytes_sender: bytes_sender,
            bytes_receiver: bytes_receiver,
            encoder: Encoder::new_with_size(
                Decoder::new_with_size(
                    BufferReader { id: id, name: "input reader".to_string(), receiver: decoder_receiver, pending: Vec::<u8>::new() },
                    INPUT_BUFFER_SIZE
                ),
                ENCODER_BUFFER_SIZE
            ),
            decoder_sender: decoder_sender,
            error: false,
        }
    }

    pub fn done(&mut self) {
        self.is_done = true;
        info!("Transaction {} is set as done for uri: {}", self.id, self.uri);
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
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
            },
            Err(SendError(sent)) => {
                error!("Failed to send {} bytes", sent.len());
            }
        }
    }
}
