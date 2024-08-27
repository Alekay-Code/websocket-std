use crate::ffi::common::WSSConfig_t;
use super::super::super::sync::client::{Config, WSEvent as RWSEvent, WSClient};
use std::ffi::{c_char, c_ulong, c_void, CStr};
use std::alloc::{alloc, Layout};
use std::mem;
use std::ptr;
use std::str;
use super::super::common;

#[no_mangle]
extern "C" fn wssclient_new<'a>() -> *mut WSClient<'a, *mut c_void> {
    // Box doesn't return a Result type, that the reason to use layout, to check if the system
    // gave me memory to store the client.
    let size = mem::size_of::<WSClient<*mut c_void>>();
    let aling = std::mem::align_of::<WSClient<*mut c_void>>();
    let layout = Layout::from_size_align(size, aling);

    if layout.is_err() {
        return std::ptr::null_mut();
    }

    let ptr = unsafe { alloc(layout.unwrap()) };
    let client = WSClient::<*mut c_void>::new();

    unsafe {
        ptr::copy_nonoverlapping(&client, ptr as *mut WSClient<*mut c_void>, 1);
    }

    ptr as *mut WSClient<*mut c_void>
}

// TODO client init now return ws status
#[no_mangle]
unsafe extern "C" fn wssclient_init<'a> (
    client: *mut WSClient<'a, *mut c_void>,
    url: *const c_char,
    config: WSSConfig_t,
) {
    let client = &mut *client;

    let url = str::from_utf8(CStr::from_ptr(url).to_bytes()).unwrap();
    let len = config.protocols.len;

    println!("Rust protocol len: {}", len);

    let protocols_ref = unsafe { (&*config.protocols.p).as_ptr() };
    for i in 0..config.protocols.len as usize {
        let pointer = *protocols_ref.wrapping_add(i);
        let protocol = str::from_utf8(CStr::from_ptr(pointer).to_bytes()).unwrap();
        client.add_protocol(protocol);
    }

    let callback = if !config.callback.is_null() { 
        let config_callback: fn(&mut WSClient<'a, *mut c_void>, &RWSEvent, Option<*mut c_void>) = mem::transmute(config.callback);
        Some(config_callback)
    } else { 
        None
    };

    let config = Config { callback, data: None, protocols: &[], certs: &[] };

    client.init(url, Some(config));
}

#[no_mangle]
unsafe extern "C" fn wssclient_loop<'a>(client: *mut WSClient<'a, *mut c_void>) -> common::WSStatus {
    let client = &mut *client;

    match client.event_loop() {
        Ok(_) => {}
        Err(e) => {
            return common::rust_error_to_c_error(e);
        } 
    }

    common::WSStatus::OK
}

#[no_mangle]
unsafe extern "C" fn wssclient_send<'a>(client: *mut WSClient<'a, *mut c_void>, message: *const c_char) {
    let msg = str::from_utf8(CStr::from_ptr(message).to_bytes()).unwrap();
    let client = &mut *client;
    client.send(msg);
}

#[no_mangle]
extern "C" fn wssclient_drop<'a>(client: *mut WSClient<'a, *mut c_void>) {
    // Create a box from the raw pointer, at the end of the function the client will be dropped and the memory will be free.
    unsafe {
        let c = *Box::from_raw(client);
        drop(c); 
    }
}

#[no_mangle]
extern "C" fn wssclient_protocol<'a>(client: *mut WSClient<'a, *mut c_void>) -> *const u8 {
    unsafe {
        let client = &mut *client;
        let protocol = client.protocol();
        if protocol.is_none() { return ptr::null(); }

        let protocol = protocol.unwrap();
        protocol.as_ptr()
    }
}
