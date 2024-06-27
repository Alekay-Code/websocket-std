use std::thread;
use std::time::Instant;
use websocket_std::sync::client::{WSClient, Config, Reason, WSEvent};
use websocket_std::result::WebSocketResult;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Data {
    count: usize,
}

// You can use any of the sync mechanism in std, for instance RWLock
type WSData = Arc<Mutex<Data>>;
type WebSocket<'a> = WSClient<'a, WSData>;

fn websocket_handler(ws: &mut WebSocket, event: &WSEvent, data: Option<WSData>) {
    match event {
        WSEvent::ON_CONNECT(msg) => on_connect(ws, msg, data),
        WSEvent::ON_TEXT(msg) => on_message(ws, msg, data),
        WSEvent::ON_CLOSE(reason) => on_close(reason, data)
    }
}

fn on_message(_ws: &mut WebSocket, _msg: &String, data: Option<WSData>) {
    let data = data.unwrap();
    let mut data = data.lock().unwrap();
    data.count += 1;
    println!("[SERVER]: {}", _msg);
}

fn on_connect(ws: &mut WebSocket, _msg: &Option<String>, _data: Option<WSData>) {
    println!("Connected");
    let protocol = if ws.protocol().is_some() { ws.protocol().unwrap() } else { "--" };
    println!("Accepted protocol: {protocol}");

    if let Some(msg) = _msg {
        println!("Message received on connect: {}", msg);
    }

    ws.send("Hello world");
}

fn on_close(reason: &Reason, _data: Option<WSData>) {
    let mut _who_closed = "";
    let mut _code = 0u16;

    match reason {
        Reason::SERVER_CLOSE(c) => {
            _who_closed = "server";
            _code = *c;
        },

        Reason::CLIENT_CLOSE(c) => {
            _who_closed = "client";
            _code = *c;
        }
    }

    println!("Connection closed by {_who_closed}, code: {_code}");
}

fn worker(client: &mut WebSocket) {
    let start = Instant::now();
    loop {
        if start.elapsed().as_secs() >= 10 { break }
        let result = client.event_loop(); 
        if result.is_err() {
            print!("{}", result.unwrap_err());
            break;
        }
    }
}

fn main() -> WebSocketResult<()> {
    let url = "ws://localhost:3000";
    let data: WSData = Arc::new(Mutex::new(Data { count: 0 }));

    let config = Config {
        callback: Some(websocket_handler), 
        data: Some(data.clone()),
        protocols: &["chat", "superchat"],
        certs: &[]
    };
    
    println!("Connecting to {}", url);

    let mut c1 = WSClient::new();
    let mut c2 = WSClient::new();


    if let Some(protocol) = c1.protocol() {
        println!("Accepted protocol: {}", protocol); 
    }

    c1.init(url, Some(config.clone()))?;
    c2.init(url, Some(config.clone()))?;

    let t1 = thread::spawn(move || worker(&mut c1));
    let t2 =thread::spawn(move || worker(&mut c2));

    let _ = t1.join();
    let _ = t2.join();

    println!("Total messages received: {}", data.lock().unwrap().count);
    Ok(())
}