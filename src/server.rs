//! WebSocket server running off the main thread.
//!
//! Each connection gets its own thread. Incoming text frames are forwarded to
//! the main thread through the inbox channel; replies come back through a
//! per-connection outbox that the connection thread drains between reads.

use crate::log::log;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;
use std::time::Duration;
use tungstenite::{Error as WsError, Message};

pub type ConnId = u64;

pub enum Inbound {
    Connected { conn: ConnId, outbox: Sender<String> },
    Message { conn: ConnId, text: String },
    Disconnected { conn: ConnId },
}

const PORTS: std::ops::RangeInclusive<u16> = 9670..=9679;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Binds a localhost port and starts accepting connections. Returns the port.
pub fn start(inbox: Sender<Inbound>) -> std::io::Result<u16> {
    let listener = bind()?;
    let port = listener.local_addr()?.port();
    thread::Builder::new()
        .name("reasock-accept".into())
        .spawn(move || accept_loop(listener, inbox))?;
    Ok(port)
}

fn bind() -> std::io::Result<TcpListener> {
    if let Some(port) = std::env::var("REASOCK_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    {
        return TcpListener::bind(("127.0.0.1", port));
    }
    let mut last_error = None;
    for port in PORTS {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("no ports to try")))
}

fn accept_loop(listener: TcpListener, inbox: Sender<Inbound>) {
    let mut next_conn: ConnId = 1;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let conn = next_conn;
                next_conn += 1;
                let inbox = inbox.clone();
                let spawned = thread::Builder::new()
                    .name(format!("reasock-conn-{conn}"))
                    .spawn(move || serve(conn, stream, inbox));
                if let Err(error) = spawned {
                    log!("conn {conn}: failed to spawn thread: {error}");
                }
            }
            Err(error) => log!("accept failed: {error}"),
        }
    }
}

fn serve(conn: ConnId, stream: TcpStream, inbox: Sender<Inbound>) {
    let _ = stream.set_nodelay(true);
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(error) => {
            log!("conn {conn}: handshake failed: {error}");
            return;
        }
    };
    // A short read timeout turns the blocking socket into a poll loop so the
    // same thread can also flush the outbox.
    let _ = ws.get_ref().set_read_timeout(Some(POLL_INTERVAL));

    let (outbox, outgoing) = mpsc::channel::<String>();
    if inbox.send(Inbound::Connected { conn, outbox }).is_err() {
        return;
    }
    log!("conn {conn}: connected");

    'session: loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                let text = text.as_str().to_owned();
                if inbox.send(Inbound::Message { conn, text }).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(WsError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => break,
            Err(error) => {
                log!("conn {conn}: read failed: {error}");
                break;
            }
        }

        loop {
            match outgoing.try_recv() {
                Ok(text) => {
                    if let Err(error) = ws.send(Message::text(text)) {
                        log!("conn {conn}: send failed: {error}");
                        break 'session;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    break 'session;
                }
            }
        }
        // Pushes out queued control frames (pongs, close replies).
        let _ = ws.flush();
    }

    log!("conn {conn}: disconnected");
    let _ = inbox.send(Inbound::Disconnected { conn });
}
