use std::fs::File;
use std::io::Write;

#[no_mangle]
pub extern "C" fn send() {
    let mut file = File::create("/tmp/analyzer.txt");
    file.expect("Unable to open file").write_all(b"We're alive!\n").ok();
}
