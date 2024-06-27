use std::fmt;
use std;

// Define type for WebSocketStdResult
pub type WebSocketResult<T> = Result<T, WebSocketError>;

// Represents a WebSocket error
#[derive(Debug, PartialEq)]
pub enum WebSocketError {
    UnreachableHost,
    HandShake,
    InvalidFrame,
    InvalidUrl,
    ConnectionClose,
    DecodingFromUTF8, 
    IOError,
}

// New Errors:
// --------------
// - UnreachableHost
// - Handshacke
// - DataFrame
// - ConnectionClose
// - IOError (timeout, brokenpipe, addrInuse.....)

// ProtocolError --> DataFrameError

impl fmt::Display for WebSocketError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str("WebSocketError: ")?;
        match self {
            WebSocketError::UnreachableHost => fmt.write_str("Unreachable host"),
            WebSocketError::HandShake => fmt.write_str("Error performing initial handshake"),
            WebSocketError::InvalidFrame => fmt.write_str("Invalid frame received"),
            WebSocketError::InvalidUrl => fmt.write_str("Invalid URL format"),
            WebSocketError::ConnectionClose => fmt.write_str("The connection was closed"),
            WebSocketError::DecodingFromUTF8 => fmt.write_str("Error decoding from utf8"),
            WebSocketError::IOError => fmt.write_str("IOError")
        }
    }
}

impl From<std::io::Error> for WebSocketError {
    fn from(_: std::io::Error) -> Self {
        WebSocketError::IOError
    }
}
