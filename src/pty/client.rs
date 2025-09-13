use std::os::unix::net::UnixStream;
use nix::libc;

// Structure to track client information
#[derive(Debug)]
pub struct ClientInfo {
    pub stream: UnixStream,
    pub rows: u16,
    pub cols: u16,
}

impl ClientInfo {
    pub fn new(stream: UnixStream) -> Self {
        // Get initial terminal size
        let (rows, cols) = get_terminal_size().unwrap_or((24, 80));
        
        Self {
            stream,
            rows,
            cols,
        }
    }
    
    pub fn update_size(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }
}

pub fn get_terminal_size() -> Result<(u16, u16), std::io::Error> {
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok((size.ws_row, size.ws_col))
    }
}