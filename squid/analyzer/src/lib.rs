use std::fs::{
    File,
    OpenOptions
};
use std::ffi::c_void;
use std::io::Write;

#[no_mangle]
pub extern "C" fn send(buffer: c_void, size: usize) {
    let mut file = OpenOptions::new().create(true).write(true).append(true).open("/tmp/analyzer.txt");
    let mut msg = format!("Got a chunk with {} bytes.\n", size);
    file.expect("Unable to open file").write_all(msg.as_bytes()).ok();
}
