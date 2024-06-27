use std::net::TcpStream as TCP;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::collections::{HashMap, VecDeque};
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
use rustls::{self};
use rustls::pki_types::{ServerName, CertificateDer};
use webpki_roots;

// Read cert
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
    acceptable_protocols: &'ws [&'ws str],
    extensions: Vec<Extension>,
    input_events: VecDeque<Event>,
    output_events: VecDeque<Event>,
    websocket_key: String,
    close_iters: usize,                                      // Count the number of times send_message tries to execute after the close. If <= 1 don't raise error, otherwise raise ConnectionClose error 
    certs: &'ws[&'ws str]
}                                                            // The close connection depends on the order of the functions event_loop and is_connected
                        

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
            acceptable_protocols: &[],
            extensions: Vec::new(),
            close_iters: 0,
            input_events: VecDeque::new(),
            output_events: VecDeque::new(),
            websocket_key: String::new(),
            certs: &[]
        }
    }

    pub fn init(&mut self, url: &'ws str, config: Option<Config<'ws, T>>) -> WebSocketResult<()>{
        if !url::is_valid_ws_url(url) { return Err(WebSocketError::InvalidUrl); }
        self.url= url;

        if let Some(conf) = config {
            self.cb_data = conf.data;
            self.callback = conf.callback;
            self.acceptable_protocols = conf.protocols;
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
        for p in self.acceptable_protocols {
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
            let mut root_store = rustls::RootCertStore::empty();
            
            if self.certs.len() > 0 {
                for file_path in self.certs {
                    let cert_file = File::open(file_path).unwrap();
                    let mut buff_reader = BufReader::new(cert_file);
                    let mut cert = vec![];
                    buff_reader.read_to_end(&mut cert).unwrap();
                    root_store.add(CertificateDer::from(cert)).unwrap();
                }
            } else {
                root_store = rustls::RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect()
                };
            }
    
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();
    
            // Allow using SSLKEYLOGFILE
            // config.key_log = Arc::new(rustls::KeyLogFile::new());
    
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
