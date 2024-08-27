use std::net::TcpStream as TCP;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::time::{Instant, Duration};
use std::format;
use core::marker::Send;
use crate::core::net::read_into_buffer;
use crate::result::WebSocketError;
use crate::ws_basic::header::{OPCODE, FLAG};
use crate::ws_basic::frame::{DataFrame, ControlFrame, Frame, FrameKind, bytes_to_frame};
use crate::ws_basic::status_code::{WSStatus, evaulate_status_code};
use crate::core::traits::{Serialize, Parse};
use crate::core::binary::bytes_to_u16;
use super::super::result::WebSocketResult;
use crate::http::request::{Request, Method};
use crate::http::url;
use crate::http::response::Response;
use crate::ws_basic::key::{gen_key, verify_key};
use crate::extension::Extension;
use std::sync::Arc;
use super::stream::{Stream, TcpStream, TlsTcpStream};

// TLS
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName};
use webpki_roots::TLS_SERVER_ROOTS;

// For reading cert
use std::fs::File;

const DEFAULT_MESSAGE_SIZE: u64 = 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const SWITCHING_PROTOCOLS: u16 = 101;

#[allow(non_camel_case_types)]
#[derive(PartialEq)]
#[repr(C)]
enum ConnectionStatus {
    NOT_INIT,
    START_INIT,
    HANDSHAKE, 
    OPEN,
    CLIENT_WANTS_TO_CLOSE,
    SERVER_WANTS_TO_CLOSE,
    CLOSE
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum Event {
    WEBSOCKET_DATA(Box<dyn Frame>),
    HTTP_RESPONSE(Response),
    HTTP_REQUEST(Request),
    NO_DATA,
}

fn is_websocket_data(event: &Event) -> bool {
    match event {
        Event::WEBSOCKET_DATA(_) => true,
        _ => false
    }
}

#[repr(C)]
enum EventIO {
    INPUT,
    OUTPUT
}

#[derive(Clone)]
pub struct Config<'ws, T: Clone> {
    pub callback: Option<fn(&mut WSClient<'ws, T>, &WSEvent, Option<T>)>,
    pub data: Option<T>,
    pub protocols: &'ws[&'ws str],
    // List of the path of the certificate files to use for TLS for custom certificates
    // In case of empty list, Mozilla root certificates will be used 
    pub certs: &'ws[&'ws str]
}

#[allow(non_camel_case_types)]
#[repr(C)]
pub enum Reason {
    SERVER_CLOSE(u16),
    CLIENT_CLOSE(u16)
}

#[allow(non_camel_case_types)]
pub enum WSEvent { 
    ON_CONNECT(Option<String>),
    ON_TEXT(String),
    ON_CLOSE(Reason),
}

#[allow(dead_code)]
#[repr(C)]
pub struct WSClient<'ws, T: Clone> {
    url: &'ws str,
    connection_status: ConnectionStatus,
    message_size: u64,
    timeout: Duration,
    // stream: Option<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>,
    stream: Option<Box<dyn Stream>>,
    recv_storage: Vec<u8>,                                   // Storage to keep the bytes received from the socket (bytes that didn't use to create a frame)
    recv_data: Vec<u8>,                                      // Store the data received from the Frames until the data is completelly received
    cb_data: Option<T>,
    callback: Option<fn(&mut Self, &WSEvent, Option<T>)>,
    protocol: Option<String>,
    //acceptable_protocols: Vec<String>,
    acceptable_protocols: Vec<String>,
    extensions: Vec<Extension>,
    input_events: VecDeque<Event>,
    output_events: VecDeque<Event>,
    websocket_key: String,
    certs: &'ws[&'ws str]
} 
                        

impl<'ws, T> WSClient<'ws, T> where T: Clone {
    pub fn new() -> Self {
        WSClient { 
            url: "",
            connection_status: ConnectionStatus::NOT_INIT, 
            message_size: DEFAULT_MESSAGE_SIZE, 
            stream: None, 
            recv_storage: Vec::new(), 
            recv_data: Vec::new(), 
            timeout: DEFAULT_TIMEOUT, 
            cb_data: None,
            callback: None,
            protocol: None,
            acceptable_protocols: Vec::new(),
            extensions: Vec::new(),
            input_events: VecDeque::new(),
            output_events: VecDeque::new(),
            websocket_key: String::new(),
            certs: &[]
        }
    }

    pub fn add_protocol(&mut self, protocol: &str) {
        let s = String::from(protocol);
        self.acceptable_protocols.push(s);
    }

    pub fn init(&mut self, url: &'ws str, config: Option<Config<'ws, T>>) -> WebSocketResult<()>{
        if !url::is_valid_ws_url(url) { return Err(WebSocketError::InvalidUrl); }
        self.url= url;

        if let Some(conf) = config {
            self.cb_data = conf.data;
            self.callback = conf.callback;
            
            for p in conf.protocols {
                let s = String::from_str(*p).unwrap();
                self.acceptable_protocols.push(s);
            }

            self.certs = conf.certs;
        }


        self.connection_status = ConnectionStatus::START_INIT;

        Ok(())
    }

    fn start_init(&mut self) -> WebSocketResult<()> {
        let (host, port) = url::get_address(self.url);
        let socket = TCP::connect(format!("{}:{}", host, port));
        if socket.is_err() { return Err(WebSocketError::UnreachableHost)} 

        let sec_websocket_key = gen_key();
        
        let mut headers: HashMap<String, String> = HashMap::from([
            (String::from("Upgrade"), String::from("websocket")),
            (String::from("Connection"), String::from("Upgrade")),
            (String::from("Sec-WebSocket-Key"), sec_websocket_key.clone()),
            (String::from("Sec-WebSocket-Version"), String::from("13")),
            (String::from("User-agent"), String::from("rust-websocket-std")),
            (String::from("Host"), host.clone())
        ]);

        // Add protocols to request
        let mut protocols_value = String::new();
        for p in &self.acceptable_protocols {
            protocols_value.push_str(p);
            protocols_value.push_str(", ");
        }
        if self.acceptable_protocols.len() > 0 {
            headers.insert(String::from("Sec-WebSocket-Protocol"), (&(protocols_value)[0..protocols_value.len()-2]).to_string());
        }

        let path = url::get_path(self.url);
        let request = Request::new(Method::GET, path, "HTTP/1.1", Some(headers));
        
        self.output_events.push_front(Event::HTTP_REQUEST(request)); // Push front, because the client could execute send before init (store the frames to send to do it later)
        self.websocket_key = sec_websocket_key;
        let socket = socket.unwrap();
        socket.set_nonblocking(true)?;

        if url::is_secure(self.url)  {
            let mut root_store = RootCertStore::empty();
let cert = vec![48, 130, 5, 123, 48, 130, 3, 99, 2, 20, 2, 167, 72, 200, 160, 7, 246, 250, 54, 182, 5, 247, 17, 148, 105, 9, 93, 17, 79, 159, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 48, 122, 49, 11, 48, 9, 6, 3, 85, 4, 6, 19, 2, 85, 83, 49, 19, 48, 17, 6, 3, 85, 4, 8, 12, 10, 67, 97, 108, 105, 102, 111, 114, 110, 105, 97, 49, 22, 48, 20, 6, 3, 85, 4, 7, 12, 13, 83, 97, 110, 32, 70, 114, 97, 110, 99, 105, 115, 99, 111, 49, 19, 48, 17, 6, 3, 85, 4, 10, 12, 10, 77, 121, 32, 99, 111, 109, 112, 97, 110, 121, 49, 20, 48, 18, 6, 3, 85, 4, 11, 12, 11, 77, 121, 32, 68, 105, 118, 105, 115, 105, 111, 110, 49, 19, 48, 17, 6, 3, 85, 4, 3, 12, 10, 77, 121, 32, 82, 111, 111, 116, 32, 67, 65, 48, 30, 23, 13, 50, 52, 48, 54, 50, 56, 49, 50, 48, 49, 51, 51, 90, 23, 13, 51, 52, 48, 54, 50, 54, 49, 50, 48, 49, 51, 51, 90, 48, 122, 49, 11, 48, 9, 6, 3, 85, 4, 6, 19, 2, 85, 83, 49, 19, 48, 17, 6, 3, 85, 4, 8, 12, 10, 67, 97, 108, 105, 102, 111, 114, 110, 105, 97, 49, 22, 48, 20, 6, 3, 85, 4, 7, 12, 13, 83, 97, 110, 32, 70, 114, 97, 110, 99, 105, 115, 99, 111, 49, 19, 48, 17, 6, 3, 85, 4, 10, 12, 10, 77, 121, 32, 99, 111, 109, 112, 97, 110, 121, 49, 20, 48, 18, 6, 3, 85, 4, 11, 12, 11, 77, 121, 32, 68, 105, 118, 105, 115, 105, 111, 110, 49, 19, 48, 17, 6, 3, 85, 4, 3, 12, 10, 77, 121, 32, 82, 111, 111, 116, 32, 67, 65, 48, 130, 2, 34, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 3, 130, 2, 15, 0, 48, 130, 2, 10, 2, 130, 2, 1, 0, 186, 124, 93, 8, 24, 222, 114, 245, 123, 232, 56, 52, 146, 140, 12, 144, 138, 129, 220, 137, 188, 53, 135, 12, 106, 224, 166, 5, 255, 104, 165, 21, 123, 197, 138, 207, 192, 104, 216, 151, 107, 253, 161, 21, 116, 61, 91, 170, 224, 40, 173, 39, 213, 127, 76, 143, 59, 89, 168, 245, 178, 51, 23, 165, 30, 50, 185, 20, 137, 59, 180, 176, 186, 189, 87, 28, 44, 66, 18, 76, 130, 38, 246, 63, 129, 197, 140, 89, 72, 104, 171, 97, 49, 15, 251, 65, 11, 88, 160, 192, 222, 246, 23, 76, 56, 76, 49, 106, 127, 51, 148, 113, 226, 134, 181, 247, 5, 102, 254, 93, 101, 129, 155, 243, 255, 118, 167, 253, 93, 251, 226, 190, 56, 235, 33, 133, 230, 8, 76, 160, 19, 60, 136, 96, 239, 219, 170, 170, 162, 136, 215, 183, 149, 15, 111, 188, 94, 92, 237, 226, 115, 156, 160, 85, 83, 204, 4, 5, 243, 187, 154, 82, 175, 201, 147, 228, 248, 93, 61, 41, 182, 71, 84, 196, 43, 62, 111, 110, 187, 55, 168, 116, 116, 188, 50, 107, 103, 51, 15, 0, 243, 73, 101, 75, 218, 73, 153, 227, 29, 103, 4, 201, 123, 166, 251, 165, 14, 172, 31, 26, 180, 121, 192, 180, 254, 239, 186, 16, 29, 210, 146, 17, 231, 165, 111, 128, 149, 89, 244, 221, 162, 183, 24, 101, 22, 55, 102, 106, 205, 231, 237, 212, 109, 63, 13, 248, 116, 153, 227, 3, 52, 81, 83, 50, 202, 65, 157, 243, 64, 250, 26, 155, 67, 131, 208, 227, 192, 41, 237, 142, 203, 13, 13, 238, 25, 26, 22, 146, 22, 207, 64, 30, 134, 60, 249, 218, 90, 40, 34, 140, 61, 165, 216, 134, 86, 64, 152, 28, 94, 52, 163, 194, 4, 228, 184, 26, 101, 231, 28, 21, 159, 130, 130, 139, 232, 173, 107, 249, 95, 130, 105, 49, 186, 153, 78, 181, 18, 43, 145, 76, 0, 143, 255, 228, 28, 158, 253, 69, 227, 238, 23, 28, 14, 17, 137, 234, 225, 107, 76, 167, 143, 245, 31, 82, 140, 222, 187, 241, 196, 96, 99, 110, 36, 43, 73, 46, 122, 76, 123, 81, 105, 63, 210, 137, 37, 10, 203, 57, 131, 244, 34, 3, 53, 121, 7, 186, 249, 13, 130, 191, 226, 165, 184, 48, 44, 177, 107, 93, 134, 37, 209, 10, 107, 13, 37, 99, 21, 163, 175, 26, 5, 220, 51, 157, 80, 184, 50, 249, 245, 179, 139, 8, 128, 78, 237, 154, 230, 150, 46, 130, 67, 196, 175, 92, 113, 55, 77, 23, 235, 180, 196, 98, 159, 33, 124, 42, 32, 109, 149, 83, 49, 128, 111, 88, 166, 21, 94, 238, 99, 171, 73, 65, 72, 89, 208, 94, 167, 194, 21, 109, 121, 4, 187, 251, 164, 252, 254, 1, 14, 183, 103, 223, 194, 230, 154, 222, 94, 130, 205, 235, 37, 104, 116, 41, 124, 49, 113, 162, 89, 106, 241, 59, 2, 3, 1, 0, 1, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 3, 130, 2, 1, 0, 94, 70, 175, 129, 48, 235, 121, 13, 75, 76, 40, 115, 68, 220, 55, 29, 66, 106, 118, 148, 88, 80, 115, 201, 188, 71, 171, 29, 91, 202, 33, 201, 102, 183, 90, 5, 122, 108, 67, 67, 56, 250, 164, 173, 227, 249, 192, 57, 112, 186, 156, 64, 165, 135, 183, 154, 138, 109, 254, 16, 13, 167, 234, 43, 180, 172, 145, 105, 161, 231, 158, 39, 227, 177, 168, 196, 229, 204, 173, 115, 2, 28, 88, 2, 241, 27, 118, 107, 120, 132, 94, 82, 245, 56, 32, 196, 118, 171, 150, 116, 14, 112, 0, 94, 222, 65, 189, 8, 179, 223, 5, 109, 192, 105, 49, 64, 30, 216, 80, 159, 115, 161, 14, 236, 105, 79, 149, 184, 148, 193, 191, 21, 136, 213, 148, 253, 156, 157, 124, 129, 244, 119, 73, 46, 73, 74, 237, 67, 242, 51, 64, 55, 82, 180, 114, 21, 156, 221, 12, 99, 186, 93, 141, 28, 153, 242, 212, 11, 4, 178, 55, 190, 222, 10, 13, 138, 232, 53, 253, 100, 69, 212, 199, 3, 150, 27, 58, 173, 50, 173, 208, 175, 11, 164, 136, 78, 212, 177, 164, 73, 133, 74, 180, 145, 21, 177, 132, 176, 160, 144, 107, 127, 48, 86, 18, 68, 203, 152, 34, 20, 32, 161, 230, 240, 18, 112, 164, 147, 162, 167, 212, 35, 38, 194, 249, 93, 213, 80, 99, 4, 241, 182, 163, 75, 70, 246, 230, 103, 199, 58, 48, 3, 156, 211, 54, 182, 91, 181, 122, 146, 178, 78, 1, 196, 41, 147, 118, 114, 84, 214, 64, 155, 244, 244, 79, 239, 99, 101, 62, 139, 10, 60, 229, 106, 235, 24, 134, 119, 210, 142, 3, 59, 158, 57, 171, 125, 127, 233, 239, 155, 73, 42, 78, 62, 15, 201, 113, 92, 97, 67, 229, 210, 73, 89, 236, 233, 160, 35, 92, 89, 227, 65, 54, 11, 21, 157, 189, 46, 156, 146, 148, 248, 119, 11, 88, 214, 190, 56, 225, 178, 125, 176, 226, 173, 47, 147, 242, 61, 236, 242, 114, 100, 108, 194, 112, 0, 108, 9, 223, 113, 180, 196, 122, 183, 149, 214, 103, 153, 153, 149, 147, 138, 26, 233, 253, 36, 70, 83, 197, 180, 183, 119, 37, 196, 0, 246, 55, 154, 20, 31, 161, 109, 249, 150, 229, 56, 64, 113, 185, 229, 145, 79, 205, 70, 253, 80, 80, 60, 141, 249, 73, 14, 46, 114, 95, 223, 163, 217, 125, 237, 173, 234, 76, 30, 183, 36, 239, 127, 50, 144, 171, 168, 149, 3, 64, 116, 235, 139, 77, 225, 76, 149, 146, 9, 102, 219, 151, 247, 148, 76, 249, 215, 65, 139, 55, 108, 21, 236, 105, 97, 159, 137, 37, 7, 6, 205, 92, 26, 81, 13, 235, 17, 75, 78, 243, 33, 247, 72, 139, 219, 72, 72, 162, 84, 214, 3, 120, 198, 82, 126, 0, 180, 210, 74, 40, 232, 217, 169, 90, 60, 93, 165, 55, 147, 90, 2, 0, 96, 199, 84, 53, 37];
            if self.certs.len() > 0 {
                let c = CertificateDer::from(cert);
                root_store.add(c).unwrap();
                // for file_path in self.certs {
                //     let cert_file = File::open(file_path).unwrap();
                //     let mut buff_reader = BufReader::new(cert_file);
                //     let mut cert = vec![];
                //     buff_reader.read_to_end(&mut cert).unwrap();
                //     root_store.add(CertificateDer::from(cert)).unwrap();
                // }
            } else {
                root_store = rustls::RootCertStore {
                    roots: TLS_SERVER_ROOTS.iter().cloned().collect()
                };
            }
    
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();
    
            let server_name: ServerName = host.try_into().unwrap();
            let conn = rustls::ClientConnection::new(Arc::new(config), server_name).unwrap();
            
            let stream: rustls::StreamOwned<rustls::ClientConnection, TCP> = rustls::StreamOwned::new(conn, socket);
            let stream = TlsTcpStream::new(stream);
            self.stream = Some(Box::new(stream));

        } else {
            let stream = TcpStream::new(socket);
            self.stream = Some(Box::new(stream));
        }


        self.connection_status = ConnectionStatus::HANDSHAKE;
            
        Ok(())
    }

    // Returns the protocol accepted by the server
    pub fn protocol(&self) -> Option<&str> {
        if self.protocol.is_none() { return None };
        return Some(self.protocol.as_ref().unwrap().as_str());
    }

    pub fn set_message_size(&mut self, size: u64) {
        self.message_size = size;
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn send(&mut self, payload: &str) {
        // If connection is close do nothing
        if self.connection_status == ConnectionStatus::CLOSE { return }
        let mut data_sent = 0;
        let mut _i: usize = 0;

        while data_sent < payload.len() {
            _i = data_sent + self.message_size as usize; 
            if _i >= payload.len() { _i = payload.len() };
            let payload_chunk = payload[data_sent.._i].as_bytes();
            let flag = if data_sent + self.message_size as usize >= payload.len() { FLAG::FIN } else { FLAG::NOFLAG };
            let code = if data_sent == 0 { OPCODE::TEXT } else { OPCODE::CONTINUATION };
            let frame = DataFrame::new(flag, code, payload_chunk.to_vec(), true, None);
            self.output_events.push_back(Event::WEBSOCKET_DATA(Box::new(frame)));
            data_sent += self.message_size as usize;
        }
    }

    pub fn event_loop(& mut self) -> WebSocketResult<()> {
        if self.connection_status == ConnectionStatus::NOT_INIT { return Ok(()) }
        if self.connection_status == ConnectionStatus::START_INIT { return self.start_init()}
        if self.connection_status == ConnectionStatus::CLOSE { return Err(WebSocketError::ConnectionClose) }
    
        let event = self.read_bytes_from_socket()?;
        self.insert_input_event(event);
        
        let in_event = self.input_events.pop_front();     
        // Check that the message taken from the queue is not a websocket event and the state of the websocket is different
        // - if the state is HANDSHAKE dont pop an event if is a websocket event
        let out_event = self.pop_output_event();

        if in_event.is_some() { self.handle_event(in_event.unwrap(), EventIO::INPUT)? };
        if out_event.is_some() { self.handle_event(out_event.unwrap(), EventIO::OUTPUT)? };

        return Ok(())
    }

    fn pop_output_event(&mut self) -> Option<Event> {
        let mut out_event = self.output_events.pop_front();
        if out_event.is_some() &&
        self.connection_status == ConnectionStatus::HANDSHAKE && 
        is_websocket_data(out_event.as_ref().unwrap())
            {
                self.output_events.push_front(out_event.unwrap());
                out_event = None;
            }
        return out_event;
    }

    fn handle_recv_bytes_frame(&mut self) -> WebSocketResult<Event> {
        let frame = bytes_to_frame(&self.recv_storage)?;
        if frame.is_none() { return Ok(Event::NO_DATA) };

        let (frame, offset) = frame.unwrap();

        let event = Event::WEBSOCKET_DATA(frame);
        self.recv_storage.drain(0..offset);

        Ok(event)
    }

    fn handle_recv_frame(&mut self, frame: Box<dyn Frame>) -> WebSocketResult<()> {
        match frame.kind()  {
            FrameKind::Data => { 
                if frame.get_header().get_flag() != FLAG::FIN {
                    self.recv_data.extend_from_slice(frame.get_data());
                }

                if self.callback.is_some() {
                    let callback = self.callback.unwrap();

                    let res = String::from_utf8(frame.get_data().to_vec());
                    if res.is_err() { return Err(WebSocketError::DecodingFromUTF8) }
                    
                    let msg = res.unwrap();

                    // Message received in a single frame
                    if self.recv_data.is_empty() {
                        callback(self, &WSEvent::ON_TEXT(msg), self.cb_data.clone());

                    // Message from a multiples frames     
                    } else {
                        let previous_data = self.recv_data.clone();
                        let res = String::from_utf8(previous_data);
                        if res.is_err() { return Err(WebSocketError::DecodingFromUTF8); }
                        
                        let mut completed_msg = res.unwrap();
                        completed_msg.push_str(msg.as_str());

                        // Send the message to the callback function
                        callback(self, &WSEvent::ON_TEXT(completed_msg), self.cb_data.clone());
                        
                        // There is 2 ways to deal with the vector data:
                        // 1 - Remove from memory (takes more time)
                        //         Creating a new vector produces that the old vector will be dropped (deallocating the memory)
                        self.recv_data = Vec::new();

                        // // 2 - Use the clear method (takes more memory because we never drop it)
                        // //         The vector does not remove memory that has already been allocated.
                        // self.recv_data.clear();
                    }
                }
                return Ok(());
            },
            FrameKind::Control => { return self.handle_control_frame(frame.as_any().downcast_ref::<ControlFrame>().unwrap()); },
            FrameKind::NotDefine => return Err(WebSocketError::InvalidFrame)
        }; 
    }

    fn handle_recv_bytes_http_response(&mut self) -> WebSocketResult<Event> {
        let response = Response::parse(&self.recv_storage);
        if response.is_err() { return Ok(Event::NO_DATA); } // TODO: Check for timeout to raise an error

        let response = response.unwrap();
        let event = Event::HTTP_RESPONSE(response);
        // TODO: Drain bytes not used in response (maybe two responses comes at the same time)
        self.recv_storage.clear();

        Ok(event)
    }

    fn handle_recv_http_response(&mut self, response: Response) -> WebSocketResult<()> {
        match self.connection_status {
            ConnectionStatus::HANDSHAKE => {
                let sec_websocket_accept = response.header("Sec-WebSocket-Accept");
            
                if sec_websocket_accept.is_none() { return Err(WebSocketError::HandShake) }
                let sec_websocket_accept = sec_websocket_accept.unwrap();
            
                // Verify Sec-WebSocket-Accept
                let accepted = verify_key(&self.websocket_key, &sec_websocket_accept);
                if !accepted {
                    return Err(WebSocketError::HandShake);
                }
            
                if response.get_status_code() == 0 || 
                   response.get_status_code() != SWITCHING_PROTOCOLS { 
                    return Err(WebSocketError::HandShake) 
                }

                self.protocol = response.header("Sec-WebSocket-Protocol");

                let mut response_msg = None;
                
                if let Some(body) = response.body() {
                   response_msg = Some(body.clone()); 
                }

                self.connection_status = ConnectionStatus::OPEN;

                if let Some(callback) = self.callback { 
                    callback(self, &WSEvent::ON_CONNECT(response_msg), self.cb_data.clone());
                }
            }
            _ =>  {} // Unreachable 
        }

        Ok(())
    }

    fn handle_send_frame(&mut self, frame: Box<dyn Frame>) -> WebSocketResult<()> {
        let sent = self.try_write(frame.serialize().as_slice())?;
        let kind = frame.kind();
        let mut status = None;

        if frame.kind() == FrameKind::Control {
            status = frame.as_any().downcast_ref::<ControlFrame>().unwrap().get_status_code();
        }

        if !sent { self.output_events.push_front(Event::WEBSOCKET_DATA(frame)) };

        if sent && kind == FrameKind::Control && self.connection_status == ConnectionStatus::SERVER_WANTS_TO_CLOSE {
            self.connection_status = ConnectionStatus::CLOSE;
            self.stream.as_mut().unwrap().shutdown()?;

            self.stream = None;

            if let Some(callback) = self.callback {
                let reason = Reason::SERVER_CLOSE(status.unwrap_or(0));
                callback(self, &WSEvent::ON_CLOSE(reason), self.cb_data.clone());
            }
        }

        Ok(())
    }

    fn handle_send_http_request(&mut self, request: Request) -> WebSocketResult<()> {
        let sent = self.try_write(request.serialize().as_slice())?;
        if !sent { 
            self.output_events.push_front(Event::HTTP_REQUEST(request)) 
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event, kind: EventIO) -> WebSocketResult<()> {

        match kind {
            EventIO::INPUT => {
                match event {
                    Event::WEBSOCKET_DATA(frame) => self.handle_recv_frame(frame)?,
                    Event::HTTP_RESPONSE(response) => self.handle_recv_http_response(response)?,
                    Event::HTTP_REQUEST(_) => {} // Unreachable
                    Event::NO_DATA => {} // Unreachable
                }
            },

            EventIO::OUTPUT => {
                match event { 
                    Event::WEBSOCKET_DATA(frame) => self.handle_send_frame(frame)?,
                    Event::HTTP_REQUEST(request) => self.handle_send_http_request(request)?,
                    Event::HTTP_RESPONSE(_) => {} // Unreachable
                    Event::NO_DATA => {} // Unreachable
                }
            }
        }

        return Ok(());
    }

    fn read_bytes_from_socket(&mut self) -> WebSocketResult<Event> {
        // TODO: Add timeout attribute to self in order to raise an error if any op overflow the time required to finish
        let mut buffer = [0u8; 1024];
        let mut reader = self.stream.as_mut().unwrap();
        let bytes_readed = read_into_buffer(&mut reader, &mut buffer)?;

        if bytes_readed > 0 {
            self.recv_storage.extend_from_slice(&buffer[0..bytes_readed]);
        }

        // Input data
        let mut event = Event::NO_DATA;
        if self.recv_storage.len() > 0 {
            match self.connection_status {
                ConnectionStatus::HANDSHAKE => event = self.handle_recv_bytes_http_response()?,
                ConnectionStatus::OPEN | ConnectionStatus::CLIENT_WANTS_TO_CLOSE | ConnectionStatus::SERVER_WANTS_TO_CLOSE => {
                    event = self.handle_recv_bytes_frame()?;
                },

                ConnectionStatus::CLOSE => {}, // Unreachable
                ConnectionStatus::NOT_INIT => {}, // Unreachable
                ConnectionStatus::START_INIT => {} // Unreachable
            };
        }
        Ok(event) 
    }

    fn insert_input_event(&mut self, event: Event) {
        match &event {
            Event::WEBSOCKET_DATA(frame) => { 
                if frame.kind() == FrameKind::Control {
                    self.input_events.push_front(event);
                } else {
                    self.input_events.push_back(event)
                }
            },

            Event::HTTP_RESPONSE(_) => self.input_events.push_back(event),
            Event::HTTP_REQUEST(_) => {} // Unreachable
            Event::NO_DATA => {}
        }
    }

    fn try_write(&mut self, bytes: &[u8]) -> WebSocketResult<bool> {
        let res = self.stream.as_mut().unwrap().write_all(bytes);
        if res.is_err(){
            let error = res.err().unwrap();

            // Try to send next iteration
            if error.kind() == ErrorKind::WouldBlock { 
                return Ok(false);

            } else {
                println!("Error: {}", error);
                println!("Error Kind: {}", error.kind());
                return Err(WebSocketError::IOError);
            }
        }
        Ok(true)
    }

    fn handle_control_frame(&mut self, frame: &ControlFrame) -> WebSocketResult<()> {
        match frame.get_header().get_opcode() {
            OPCODE::PING=> { 
                let data = frame.get_data();
                let pong_frame = ControlFrame::new(FLAG::FIN, OPCODE::PONG, None, data.to_vec(), true, None);
                self.output_events.push_front(Event::WEBSOCKET_DATA(Box::new(pong_frame)));
            },
            OPCODE::PONG => { todo!("Not implemented handle PONG") },
            OPCODE::CLOSE => {
                let data = frame.get_data();
                let status_code = &data[0..2];
                let res = bytes_to_u16(status_code);

                let status_code = if res.is_ok() { res.unwrap() } else { WSStatus::EXPECTED_STATUS_CODE.bits() };

                match self.connection_status {
                    // Server wants to close the connection
                    ConnectionStatus::OPEN => {
                        let status_code = WSStatus::from_bits(status_code);

                        let reason = &data[2..data.len()];
                        let mut status_code = if status_code.is_some() { status_code.unwrap() } else { WSStatus::PROTOCOL_ERROR };
                        
                        let (error, _) = evaulate_status_code(status_code);
                        if error { status_code = WSStatus::PROTOCOL_ERROR }

                        // Enqueue close frame to response to the server
                        self.output_events.clear();
                        self.input_events.clear();
                        let close_frame = ControlFrame::new(FLAG::FIN, OPCODE::CLOSE, Some(status_code.bits()), reason.to_vec(), true, None);
                        self.output_events.push_front(Event::WEBSOCKET_DATA(Box::new(close_frame)));

                        self.connection_status = ConnectionStatus::SERVER_WANTS_TO_CLOSE;
                        
                        // TODO: Create and on close cb to handle this situation, send the status code an the reason
                    },
                    ConnectionStatus::CLIENT_WANTS_TO_CLOSE => {
                        // TODO: ?
                        // Received a response to the client close handshake
                        // Verify the status of close handshake
                        self.connection_status = ConnectionStatus::CLOSE;
                        self.stream.as_mut().unwrap().shutdown()?;
                        
                        if let Some(callback) = self.callback {
                            let reason = Reason::CLIENT_CLOSE(frame.get_status_code().unwrap());
                            callback(self, &WSEvent::ON_CLOSE(reason), self.cb_data.clone());
                        }
                    },
                    ConnectionStatus::SERVER_WANTS_TO_CLOSE => {}  // Unreachable  
                    ConnectionStatus::CLOSE => {}                  // Unreachable
                    ConnectionStatus::HANDSHAKE => {}              // Unreachable
                    ConnectionStatus::NOT_INIT => {}               // Unreachable
                    ConnectionStatus::START_INIT => {}             // Unreachable
                }
            },
            _ => return Err(WebSocketError::InvalidFrame)
        }

        Ok(())
    }
}

impl<'ws, T> Drop for WSClient<'ws, T> where T: Clone {
    fn drop(&mut self) {
        if self.connection_status != ConnectionStatus::NOT_INIT &&
            self.connection_status != ConnectionStatus::HANDSHAKE &&
            self.connection_status != ConnectionStatus::CLOSE &&
            self.stream.is_some() {

                let msg = "Done";
                let status_code: u16 = 1000;
                let close_frame = ControlFrame::new(FLAG::FIN, OPCODE::CLOSE, Some(status_code), msg.as_bytes().to_vec(), true, None);
        
                // Add close frame at the end of the queue.
                // Clear both queues
                self.output_events.clear();
                self.input_events.clear();
                self.output_events.push_back(Event::WEBSOCKET_DATA(Box::new(close_frame)));
                self.connection_status = ConnectionStatus::CLIENT_WANTS_TO_CLOSE;
        
                let timeout = Instant::now();
        
                // Process a response for all the events and confirm that the connection was closed.
                while self.connection_status != ConnectionStatus::CLOSE {
                    if timeout.elapsed().as_secs() >= self.timeout.as_secs() { break } // Close handshake timeout.
                    let result = self.event_loop();
                    if result.is_ok() { continue }
                    let err = result.err().unwrap();

                    // TODO: Decide what to do if an error ocurred while consuming the rest of the messages
                    match err {
                        _ => { break }
                    }
        
                    }
                let _ = self.stream.as_mut().unwrap().shutdown(); // Ignore result from shutdown method.
            }
        }
}

unsafe impl<'ws, T> Send for WSClient<'ws, T> where T: Clone {}
