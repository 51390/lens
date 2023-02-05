use std::fs::OpenOptions;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::sync::Once;
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

#[no_mangle]
pub extern "C" fn send(id: i64, chunk: *const c_void, size: usize) {
    let buffers = get_buffers();
    let ptr = chunk as *const u8;
    match buffers.transactions.get_mut(&id) {
        Some(buffer) => unsafe {
            buffer.extend(std::slice::from_raw_parts(ptr, size));
            //buffers.transactions.insert(id, buffer);
        },
        None => drop(buffers.transactions.insert(id, Vec::new())),
    }

    let filename = format!("/tmp/request-body-{}.log", id);
    let file = OpenOptions::new().create(true).write(true).append(true).open(filename);
    let content = unsafe { std::slice::from_raw_parts(ptr, size) };
    file.expect("Unable to open file.").write_all(content).ok();
}
