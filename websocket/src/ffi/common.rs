use super::super::sync::client::{WSEvent as RWSEvent, Reason};
use std::ffi::{c_void, CString, c_char, c_ulong};
use crate::result::WebSocketError;
use std::ptr;

#[repr(C)]
#[allow(non_camel_case_types)]
enum WSEvent {
    ON_CONNECT,
    ON_TEXT,
    ON_CLOSE
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct WSEvent_t {
    event: WSEvent,
    value: *const c_void
}

#[repr(C)]
#[allow(non_camel_case_types)]
enum WSReason {
    SERVER_CLOSED,
    CLIENT_CLOSED
}

#[repr(C)]
#[allow(non_camel_case_types)]
struct WSReason_t {
    reason: WSReason,
    status: u16
}

#[repr(C)]
#[derive(Debug, Clone)]
pub enum WSStatus { 
    OK,
    UnreachableHost,
    HandShake,
    InvalidFrame,
    InvalidUrl,
    ConnectionClose,
    DecodingFromUTF8,
    IOError,
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct Protocols_t {
    pub len: c_ulong,
    pub p: *const[*const c_char]
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct WSSConfig_t {
    pub callback: *const c_void,
    pub protocols: Protocols_t
}

pub fn rust_error_to_c_error(error: WebSocketError) -> WSStatus {
    match error {
        WebSocketError::UnreachableHost => WSStatus::UnreachableHost,
        WebSocketError::HandShake => WSStatus::HandShake,
        WebSocketError::InvalidFrame => WSStatus::InvalidFrame,
        WebSocketError::InvalidUrl=> WSStatus::InvalidUrl,
        WebSocketError::ConnectionClose => WSStatus::ConnectionClose,
        WebSocketError::DecodingFromUTF8 => WSStatus::DecodingFromUTF8,
        WebSocketError::IOError => WSStatus::IOError
    }
}

#[no_mangle]
pub extern "C" fn from_rust_event(event: &RWSEvent) -> WSEvent_t {
    match event {
        RWSEvent::ON_CONNECT(msg) => {
            let mut value = ptr::null_mut();
            if let Some(m) = msg {
                println!("[RUST] M: {}", m);
                let m = m.replace('\0', "");
                let m = m.trim();
                let c_str = CString::new(m).map_err(|err| {
                    eprintln!("Error converting to CString: {err}");
                }).unwrap();
                value = c_str.into_raw();
            }
            WSEvent_t { event: WSEvent::ON_CONNECT, value: value as *const c_void }
        }
        , 
        RWSEvent::ON_TEXT(msg) => {
            let c_str = CString::new(msg.clone()).unwrap();
            WSEvent_t { event: WSEvent::ON_TEXT, value: c_str.into_raw() as *const c_void }
        },
        RWSEvent::ON_CLOSE(reason) => {
            let (reason, status) = match reason {
                Reason::SERVER_CLOSE(status) => (WSReason::SERVER_CLOSED, status.clone()),   
                Reason::CLIENT_CLOSE(status) => (WSReason::CLIENT_CLOSED, status.clone())
            };
            let reason = WSReason_t { reason, status };
            let reason = Box::into_raw(Box::new(reason));
            WSEvent_t { event: WSEvent::ON_CLOSE, value: reason as *const c_void } 
        }
    }
}
