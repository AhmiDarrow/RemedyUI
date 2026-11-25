//! A JSON-lines connection to an app on this machine: the agents listener and the remote
//! devtools server both speak one JSON object per line over a loopback socket.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use crate::devkit::{Result, ToolError};

/// A connection: lines are written directly and read on a thread of their own, so waiting for
/// one can time out.
#[derive(Debug)]
pub(crate) struct LineClient {
    stream: TcpStream,
    lines: Receiver<String>,
    addr: SocketAddr,
}

/// The loopback address `address` names (`127.0.0.1:PORT`, `[::1]:PORT`, `localhost:PORT`).
/// Anything else is refused: apps serve agents and devtools on this machine only.
pub(crate) fn loopback(address: &str) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = address
        .trim()
        .to_socket_addrs()
        .map_err(|e| ToolError::new(format!("`{address}` is not an address with a port: {e}")))?
        .collect();
    addrs.into_iter().find(|a| a.ip().is_loopback()).ok_or_else(|| {
        ToolError::new(format!(
            "{address} is not on this machine: apps serve agents and devtools on loopback only"
        ))
    })
}

impl LineClient {
    /// Connects to `address`, waiting at most `timeout`.
    pub(crate) fn connect(address: &str, timeout: Duration) -> Result<LineClient> {
        let addr = loopback(address)?;
        let stream = TcpStream::connect_timeout(&addr, timeout)
            .map_err(|e| ToolError::new(format!("could not connect to {addr}: {e}")))?;
        let _ = stream.set_nodelay(true);
        let read = stream.try_clone().map_err(|e| ToolError::new(format!("{addr}: {e}")))?;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("remedy-tools-lines".into())
            .spawn(move || {
                for line in BufReader::new(read).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| ToolError::new(format!("could not start reading from {addr}: {e}")))?;
        Ok(LineClient { stream, lines: rx, addr })
    }

    /// Sends one line.
    pub(crate) fn send(&mut self, line: &str) -> Result<()> {
        writeln!(self.stream, "{line}")
            .and_then(|()| self.stream.flush())
            .map_err(|e| ToolError::new(format!("the app at {} closed the connection: {e}", self.addr)))
    }

    /// The next line, `None` when none came within `timeout`.
    pub(crate) fn recv(&self, timeout: Duration) -> Result<Option<String>> {
        match self.lines.recv_timeout(timeout) {
            Ok(l) => Ok(Some(l)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ToolError::new(format!("the app at {} closed the connection", self.addr)))
            }
        }
    }

    /// The address connected to.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for LineClient {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_addresses_are_accepted() {
        assert!(loopback("127.0.0.1:4000").is_ok());
        assert!(loopback("[::1]:4000").is_ok());
        assert!(loopback("localhost:4000").is_ok());
        assert!(loopback("10.0.0.1:4000").is_err());
        assert!(loopback("no port").is_err());
    }

    #[test]
    fn lines_go_both_ways_and_waiting_times_out() {
        let Ok(server) = std::net::TcpListener::bind("127.0.0.1:0") else { unreachable!("a loopback port") };
        let Ok(addr) = server.local_addr() else { unreachable!("bound") };
        let echo = std::thread::spawn(move || {
            let Ok((s, _)) = server.accept() else { return };
            let Ok(mut out) = s.try_clone() else { return };
            let mut r = BufReader::new(s);
            let mut line = String::new();
            if r.read_line(&mut line).is_ok() {
                let _ = write!(out, "echo {line}");
            }
        });
        let Ok(mut c) = LineClient::connect(&addr.to_string(), Duration::from_secs(5)) else {
            unreachable!("connects")
        };
        assert!(c.send("hi").is_ok());
        assert_eq!(c.recv(Duration::from_secs(10)).ok().flatten().as_deref(), Some("echo hi"));
        let _ = echo.join();
        assert!(c.recv(Duration::from_secs(5)).is_err(), "the server closed");
        assert_eq!(c.addr(), addr);
    }
}
