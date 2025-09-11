use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Circular buffer for PTY output
/// Stores output generated while no client is attached
pub struct PtyBuffer {
    buffer: Arc<Mutex<VecDeque<Vec<u8>>>>,
    max_size: usize,
    total_bytes: Arc<Mutex<usize>>,
}

impl PtyBuffer {
    pub fn new(max_size: usize) -> Self {
        PtyBuffer {
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
            total_bytes: Arc::new(Mutex::new(0)),
        }
    }

    pub fn push(&self, data: &[u8]) {
        let mut buffer = self.buffer.lock().unwrap();
        let mut total = self.total_bytes.lock().unwrap();

        buffer.push_back(data.to_vec());
        *total += data.len();

        // Remove old data if we exceed max size
        while *total > self.max_size && !buffer.is_empty() {
            if let Some(old_data) = buffer.pop_front() {
                *total -= old_data.len();
            }
        }
    }

    pub fn drain_to(&self, output: &mut Vec<u8>) {
        let mut buffer = self.buffer.lock().unwrap();
        let mut total = self.total_bytes.lock().unwrap();

        while let Some(data) = buffer.pop_front() {
            output.extend_from_slice(&data);
        }

        *total = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.lock().unwrap().is_empty()
    }

    pub fn clone_handle(&self) -> Self {
        PtyBuffer {
            buffer: Arc::clone(&self.buffer),
            max_size: self.max_size,
            total_bytes: Arc::clone(&self.total_bytes),
        }
    }
}
