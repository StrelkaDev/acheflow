use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncReadExt};

const BYTES: usize = 64 * 1024;
const RECORDS: usize = 128;
const CHUNK: usize = 1024;

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<VecDeque<String>>>);
impl Logs {
    pub fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
    fn push(&self, label: &str, bytes: &[u8]) {
        let text: String = String::from_utf8_lossy(bytes)
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        let text = format!("{label}: {text}");
        let mut lines = self.0.lock().unwrap();
        lines.push_back(text);
        while lines.len() > RECORDS || lines.iter().map(String::len).sum::<usize>() > BYTES {
            lines.pop_front();
        }
    }
    pub async fn drain(&self, label: &'static str, mut reader: impl AsyncRead + Unpin) {
        let mut bytes = [0; CHUNK];
        loop {
            match reader.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(n) => self.push(label, &bytes[..n]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unbroken_and_invalid_output_is_drained_with_fixed_memory() {
        let logs = Logs::default();
        let bytes = vec![255; 256 * 1024];
        logs.drain("stderr", bytes.as_slice()).await;
        assert!(logs.snapshot().iter().map(String::len).sum::<usize>() <= BYTES);
        assert!(logs.snapshot().len() <= RECORDS);
        logs.push("stdout", b"\x1b[31merror\0\nlast");
        assert!(!logs.snapshot().last().unwrap().contains('\x1b'));
        assert!(!logs.snapshot().last().unwrap().contains('\0'));
        assert!(logs.snapshot().last().unwrap().contains("last"));
    }
}
