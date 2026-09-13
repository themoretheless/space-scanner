use humansize::{format_size, BINARY};

pub fn format_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}
