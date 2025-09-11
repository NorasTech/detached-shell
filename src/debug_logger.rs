use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref DEBUG_FILE: Mutex<Option<File>> = Mutex::new(None);
}

pub fn init_debug_log(session_id: &str) {
    let path = format!("/tmp/nds_debug_{}.log", session_id);
    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        *DEBUG_FILE.lock().unwrap() = Some(file);
    }
}

pub fn debug_log(msg: &str) {
    if let Ok(mut guard) = DEBUG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "[{}] {}", 
                chrono::Local::now().format("%H:%M:%S%.3f"), 
                msg);
            let _ = file.flush();
        }
    }
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        $crate::debug_logger::debug_log(&format!($($arg)*));
    };
}