//! The TCP listener (spec §12.3): one client at a time, on 127.0.0.1.
//!
//! Everything happens in [`Server::poll`], on the emulation thread, with
//! non-blocking sockets: the host calls it on every tick, so a request waits
//! at most one tick. There are no threads, channels or locks.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

use juku::Machine;

use crate::json::Value;
use crate::session::{Next, Session};

/// Largest message body taken, in bytes. `writeMemory` of all 64 KB is
/// about 88 KB in base64.
const MAX_BODY: usize = 1 << 20;
/// Longest header taken.
const MAX_HEADER: usize = 1024;
/// Most output kept for a client that does not read it, as a log point in a
/// busy loop can make: past it, the client is let go.
const MAX_OUTPUT: usize = 16 << 20;

struct Client {
    stream: TcpStream,
    /// Bytes read and not yet taken as messages.
    input: Vec<u8>,
    /// Bytes not yet written.
    output: Vec<u8>,
    session: Session,
    /// Close once `output` is written.
    closing: bool,
}

/// The debug server's state, as the host sees it after [`Server::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub connected: bool,
    /// The debugger has stopped the machine.
    pub stopped: bool,
}

pub struct Server {
    listener: TcpListener,
    port: u16,
    client: Option<Client>,
}

impl Server {
    /// Listens on 127.0.0.1 at `port` (0 for any free port).
    pub fn start(port: u16) -> io::Result<Server> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Server { listener, port, client: None })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Handles the messages that have come in, reports a stop the client
    /// has not heard of, writes what is waiting, and takes a new client.
    /// Call it on every tick, after running the machine.
    pub fn poll(&mut self, m: &mut Machine) -> State {
        if let Some(c) = &mut self.client {
            let open = c.read();
            while !c.closing {
                match take_message(&mut c.input) {
                    Ok(Some(body)) => {
                        // A message that is not JSON is dropped: there is no
                        // request to answer.
                        if let Ok(message) = Value::parse(&body) {
                            c.closing = c.session.handle(&message, m) == Next::End;
                        }
                    }
                    Ok(None) => break,
                    Err(()) => c.closing = true,
                }
            }
            c.session.update(m);
            let written = c.write() && c.output.len() <= MAX_OUTPUT;
            if !open || !written || c.closing && c.output.is_empty() {
                self.end(m);
            }
        }
        // After the client above, so that one who has just left makes room.
        while let Ok((stream, _)) = self.listener.accept() {
            // A client that connected and left in one go, as VS Code's
            // extension does to see whether anything listens, is let go
            // here, so as not to turn away the one coming after it.
            if self.client.as_mut().is_some_and(|c| !c.read()) {
                self.end(m);
            }
            // A second client is turned away: dropping the stream closes it.
            if self.client.is_none() && stream.set_nonblocking(true).is_ok() {
                let _ = stream.set_nodelay(true);
                m.set_debugging(true);
                self.client = Some(Client {
                    stream,
                    input: Vec::new(),
                    output: Vec::new(),
                    session: Session::new(),
                    closing: false,
                });
            }
        }
        self.state(m)
    }

    /// Runs the machine for `cycles` and polls, as the host's tick does.
    /// A stop the session goes on from (a breakpoint whose condition does not
    /// hold, a log point) does not end the run: the rest of the cycles run
    /// after it, so a log point in a busy loop costs no time. Returns the
    /// cycles run.
    pub fn run(&mut self, m: &mut Machine, cycles: u64) -> u64 {
        let mut done = 0;
        loop {
            done += m.run_cycles(cycles.saturating_sub(done));
            let stopped = |m: &Machine| m.debugger().is_some_and(|d| d.stopped().is_some());
            let was = stopped(m);
            self.poll(m);
            if done >= cycles || !was || stopped(m) {
                return done;
            }
        }
    }

    /// A snapshot the session asks the host to open, for a launch: the host
    /// opens it as the user would, and calls [`Server::snapshot_opened`].
    pub fn take_request(&mut self) -> Option<std::path::PathBuf> {
        self.client.as_mut()?.session.take_request()
    }

    /// The host has opened the snapshot asked for, or could not.
    pub fn snapshot_opened(&mut self, m: &mut Machine, opened: bool) -> State {
        if let Some(c) = &mut self.client {
            c.session.snapshot_opened(m, opened);
            c.write();
        }
        self.state(m)
    }

    /// The app's Pause: stops the machine, or lets it run on, and tells the
    /// client. Without a client it does nothing.
    pub fn set_paused(&mut self, m: &mut Machine, paused: bool) -> State {
        if let Some(c) = &mut self.client {
            if paused {
                c.session.pause(m);
            } else {
                c.session.resume(m);
            }
            c.write();
        }
        self.state(m)
    }

    /// Ends the session: the client is let go, and the machine runs on
    /// without the debugger.
    pub fn end(&mut self, m: &mut Machine) {
        if self.client.take().is_some() {
            m.set_debugging(false);
        }
    }

    fn state(&self, m: &Machine) -> State {
        State {
            connected: self.client.is_some(),
            stopped: self.client.is_some() && m.debugger().is_some_and(|d| d.stopped().is_some()),
        }
    }
}

impl Client {
    /// Reads what has arrived. False once the client has gone.
    fn read(&mut self) -> bool {
        let mut chunk = [0; 4096];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return false,
                Ok(n) => self.input.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return true,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
    }

    /// Writes as much of the session's output as the socket takes now. False
    /// if the connection failed.
    fn write(&mut self) -> bool {
        self.output.append(&mut self.session.out);
        while !self.output.is_empty() {
            match self.stream.write(&self.output) {
                Ok(0) => return false,
                Ok(n) => drop(self.output.drain(..n)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return true,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        true
    }
}

/// Takes one message from `input` if it is all there: header lines up to an
/// empty one, then as many bytes as `Content-Length` says. Err for a message
/// that cannot be read.
fn take_message(input: &mut Vec<u8>) -> Result<Option<String>, ()> {
    let mut length = None;
    let mut at = 0;
    loop {
        let Some(end) = input[at..].iter().position(|&b| b == b'\n') else {
            return if input.len() > MAX_HEADER { Err(()) } else { Ok(None) };
        };
        let line = input[at..at + end].strip_suffix(b"\r").unwrap_or(&input[at..at + end]);
        at += end + 1;
        if line.is_empty() {
            break;
        }
        let line = core::str::from_utf8(line).map_err(|_| ())?;
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("Content-Length")
        {
            length = Some(value.trim().parse::<usize>().map_err(|_| ())?);
        }
        if at > MAX_HEADER {
            return Err(());
        }
    }
    let length = length.filter(|&n| n <= MAX_BODY).ok_or(())?;
    if input.len() < at + length {
        return Ok(None);
    }
    let body = String::from_utf8(input[at..at + length].to_vec()).map_err(|_| ())?;
    input.drain(..at + length);
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let mut input = b"Content-Length: 2\r\n\r\n{}content-length:3\nX-Other: 1\n\n[1]Content-Length: 5\r\n\r\n[1".to_vec();
        assert_eq!(take_message(&mut input), Ok(Some("{}".into())));
        assert_eq!(take_message(&mut input), Ok(Some("[1]".into())));
        // Not all there yet.
        assert_eq!(take_message(&mut input), Ok(None));
        input.extend_from_slice(b",2]");
        assert_eq!(take_message(&mut input), Ok(Some("[1,2]".into())));
        assert_eq!(take_message(&mut input), Ok(None));
        assert!(input.is_empty());
    }

    #[test]
    fn bad_messages() {
        for input in [&b"\r\n{}"[..], b"Content-Length: x\r\n\r\n", b"Content-Length: 99999999\r\n\r\n"] {
            assert_eq!(take_message(&mut input.to_vec()), Err(()));
        }
        assert_eq!(take_message(&mut vec![b'x'; MAX_HEADER + 1]), Err(()));
    }
}
