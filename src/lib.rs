// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


extern crate epoll;
#[macro_use] extern crate lazy_static;
extern crate libc;
#[macro_use] extern crate log;
extern crate parking_lot;


use std::io;
use std::net::{TcpListener, ToSocketAddrs};
use std::os::unix::io::IntoRawFd;

pub use conn::Connection;

mod buf;
mod conn;
mod event_loop;
mod socket;


/// on_connect handler
static mut ON_CONNECT_OPT: Option<fn(&Connection)> = None;

/// on_recv handler
static mut ON_NEW_DATA_OPT: Option<fn(&Connection)> = None;

/// on_close handler
static mut ON_ERROR_OPT: Option<fn(&Connection, io::Error)> = None;


/// Registers a handler to be called every time a new connection has
/// been established.
pub fn register_on_connect(h: fn(conn: &Connection)) {
    unsafe { ON_CONNECT_OPT = Some(h); }
}

/// Registers a handler to be called every time there is new data available
/// from the passed connection.
pub fn register_on_recv(h: fn(conn: &Connection)) {
    unsafe { ON_NEW_DATA_OPT = Some(h); }
}

/// Registers a handler to be called every time an error has occurred for
/// the connection.
pub fn register_on_error(h: fn(conn: &Connection, err: io::Error)) {
    unsafe { ON_ERROR_OPT = Some(h); }
}

/// Starts the server and binds to the passed address.
///
/// A port number of 0 will request that the OS assigns a port.
pub fn start<A: ToSocketAddrs>(addr: A) {
    let listener_result = TcpListener::bind(addr);
    if listener_result.is_err() {
        let err = listener_result.unwrap_err();
        error!("{} during bind.", err);
        return;
    }

    let tcp_listener = listener_result.unwrap();
    info!("Bound to {}", tcp_listener.local_addr().unwrap());

    let _ = event_loop::init().map_err(|e| {
        panic!("{} during epoll creattion", e)
    });

    loop { accept_connection(&tcp_listener); }
}

fn accept_connection(tcp_listener: &TcpListener) {
    match tcp_listener.accept() {
        Ok((tcp_stream, addr)) => {
            match tcp_stream.set_nonblocking(true) {
                Ok(_) => on_new_connection(Connection {
                    socket: tcp_stream.into_raw_fd(),
                    addr: addr
                }),
                Err(err) => error!("Setting TcpStream nonblocking: {}", err)
            }
        }
        Err(e) => {
            // We need to figure out if the listener has
            // errord out or if it was an error on the part
            // of the connecting stream
            let maybe_err = tcp_listener.take_error().unwrap();
            if maybe_err.is_some() {
                let err = maybe_err.unwrap();
                // The listener itself has errord out, we would close up shop.
                error!("Listener SO_ERROR: {} with {} during accept", err, e);
                panic!();
            }
        }
    }
}

fn on_new_connection(conn: Connection) {
    info!("New connection: {:?}", conn);

    socket::init(conn.socket);
    let _ = event_loop::add_conn(conn).map_err(|e| {
        warn!("During epoll add {}", e);
    });

    unsafe {
        if ON_CONNECT_OPT.is_some() {
            let f = ON_CONNECT_OPT.as_ref().unwrap();
            f(&conn);
        }
    }
}

fn on_recv(conn: Connection) {
    unsafe {
        if ON_NEW_DATA_OPT.is_some() {
            let f = ON_NEW_DATA_OPT.as_ref().unwrap();
            f(&conn);
        }
    }
}

fn on_error(conn: Connection, err: io::Error) {
    debug!("Connection {:?} error: {}", conn, err);

    unsafe {
        if ON_ERROR_OPT.is_some() {
            let f = ON_ERROR_OPT.as_ref().unwrap();
            f(&conn, err);
        }
    }
}
