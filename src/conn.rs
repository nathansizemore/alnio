// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


use std::io;
use std::net::SocketAddr;
use std::os::unix::io::RawFd;

use event_loop;
use socket;


#[derive(Debug, Clone, Copy)]
pub struct Connection {
    pub socket: RawFd,
    pub addr: SocketAddr
}

impl Connection {
    /// Creates a new Connection.
    pub fn new(socket: RawFd, addr: SocketAddr) -> Connection {
        Connection { socket: socket, addr: addr }
    }

    /// Shuts down further transport for this socket, and
    /// informs the remote socket of disconnect.
    pub fn shutdown(&self) -> io::Result<()> {
        let _ = event_loop::del_conn(*self);
        let _ = socket::shutdown(self.socket);
        socket::close(self.socket)
    }

    /// Returns the current number of bytes available for transfer in the
    /// connections receive buffer.
    pub fn peek(&self) -> io::Result<usize> {
        socket::peek(self.socket)
    }

    /// Takes the range params out of the connections received buffer.
    ///
    /// # Notes
    ///
    /// * Atomic and thread safe operation.
    ///
    /// # Panics
    ///
    /// * If the passed range is out of bounds.
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        socket::take(self.socket, buf)
    }

    /// Transmits data to the remote connection.
    ///
    /// # Notes
    ///
    /// * Atomic and thread safe operation.
    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        socket::add_to_tx_buf(self.socket, buf)
    }
}

unsafe impl Send for Connection { }
unsafe impl Sync for Connection { }
