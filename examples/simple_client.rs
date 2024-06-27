use websocket_std::sync::client::{Config, WSClient, Reason, WSEvent};
use websocket_std::result::WebSocketResult;
use std::time;
use std::rc::Rc;
use std::cell::RefCell;

#[derive(Clone)]
struct Data {
    count: usize
}

type WSData = Rc<RefCell<Data>>; 
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
    data.borrow_mut().count += 1;
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

fn main() -> WebSocketResult<()> {
    let url: &str = "ws://localhost:3000";

    let mut client = WSClient::<WSData>::new();
    let data = Rc::new(RefCell::new(Data { count: 0 }));

    let config = Config {
        callback: Some(websocket_handler), 
        data: Some(data.clone()),
        protocols: &["chat", "superchat"],
        certs: &[],
    };

    client.init(url, Some(config))?;
    
    let start = time::Instant::now();
    loop {
        if start.elapsed() >  time::Duration::from_secs(10) { break };
        client.event_loop()?;
    }

    print!("Count: {}\n", data.borrow().count);

    Ok(())
}
