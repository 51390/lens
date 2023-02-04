use std::fs::OpenOptions;
use std::ffi::c_void;
use std::io::Write;

#[no_mangle]
pub extern "C" fn send(id: i64, buffer: *const c_void, size: usize) {
    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let mut ptr = buffer as *const u8;
    let content = unsafe { std::slice::from_raw_parts(ptr, size) };
    file.expect("Unable to open file.").write_all(content).ok();
}
