use std::io::{Read, ErrorKind};
use crate::result::{WebSocketResult, WebSocketError};

/// Copy bytes from the reader into the buffer and return amount of data read.
/// - If an EOF is reached the function will return a ``WebSocketError::Custom``because no more bytes can be read.
/// - If there's no bytes ready to read from the reader the function will return ``Ok(0)`` bytes readed and the buffer will not be modified.
/// - If there's bytes the function will return ``Ok(n)`` where 0 < n <= buf.len()
/// - Otherwise a ``WebSocketError::IOError`` will be return.
pub fn read_into_buffer<'a>(reader: &mut dyn Read, buf: &mut [u8]) -> WebSocketResult<usize> {
    match reader.read(buf) {
        Ok(amount) => {
            // Reached end of file (error in the connection)
            if amount <= 0 {
                return Err(WebSocketError::ConnectionClose)
            } else {
                return Ok(amount);
            }
        },
        Err(e) => {
            if e.kind() == ErrorKind::WouldBlock { return Ok(0) }
            return Err(WebSocketError::IOError)
            // return Err(WebSocketError::IOError(e.to_string().as_str())); // grcov-excl-line
        }
    }
}
