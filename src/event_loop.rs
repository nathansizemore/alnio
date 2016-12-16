// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


use std::collections::BTreeMap;
use std::io::{self, Error, ErrorKind};
use std::mem;
use std::os::unix::io::RawFd;
use std::thread;

use libc;
use epoll;
use epoll::{
    EPOLL_CTL_ADD,
    EPOLL_CTL_MOD,
    EPOLL_CTL_DEL,
    EPOLLET,
    EPOLLONESHOT,
    EPOLLIN,
    EPOLLOUT,
    EPOLLERR,
    EPOLLHUP,
    EPOLLRDHUP
};
use parking_lot::Mutex;

use conn::Connection;
use socket;


type ConnectionMap = Mutex<BTreeMap<RawFd, Connection>>;


static mut _epfd: RawFd = 0;

lazy_static! {
    static ref CONN_MAP: ConnectionMap = Mutex::new(BTreeMap::new());
}


pub fn init() -> io::Result<()> {
    unsafe { _epfd = try!(epoll::create(true)); }

    info!("epfd: {}", epfd());

    thread::spawn(event_loop);

    Ok(())
}

pub fn add_conn(conn: Connection) -> io::Result<()> {
    map_add(conn);

    let mut event = epoll_event(&conn);
    set_events_r(&mut event);
    epoll_add(event)
}

pub fn del_conn(conn: Connection) -> io::Result<()> {
    map_del(conn);

    let event = epoll_event(&conn);
    epoll_del(event)
}

fn event_loop() {
    info!("Starting event loop");

    const WAIT_FOREVER: i32 = -1;
    let mut buf: [libc::epoll_event; 100] = unsafe { mem::uninitialized() };
    loop {
        let r = epoll::wait(epfd(), WAIT_FOREVER, &mut buf);
        if r.is_err() {
            let err = r.unwrap_err();
            error!("{} during epoll::wait", err);
            return;
        }

        let num_events = r.unwrap();
        trace!("{} events to process", num_events);

        for x in 0..num_events {
            let e = unsafe { buf.get_unchecked(x) };
            handle_epoll_event(e);
        }
    }
}

fn handle_epoll_event(e: &libc::epoll_event) {
    if close_event(e.events) {
        handle_close_event(e);
    } else {
        if read_event(e.events) {
            handle_read_event(e);
        }

        if write_event(e.events) {
            handle_write_event(e);
        }
    }
}

fn handle_close_event(e: &libc::epoll_event) {
    let fd = e.u64 as RawFd;

    let err = {
        if socket_error(e.events) {
            match socket::get_last_error(fd) {
                Some(e) => e,
                None => Error::new(ErrorKind::Other, "Unknown SocketError")
            }
        } else {
            Error::new(ErrorKind::UnexpectedEof, "EOF")
        }
    };

    match map_get(fd) {
        Some(conn) => super::on_error(conn, err),
        None => warn!("epoll reported close event, but socket not in map")
    };
}

fn handle_read_event(e: &libc::epoll_event) {
    let fd = e.u64 as RawFd;
    match map_get(fd) {
        Some(conn) => match socket::recv(fd) {
            Ok(read) => {
                super::on_recv(conn);
                debug!("Recv {} bytes from {:?}", read, conn);
                epoll_rearm_r(conn);
            }
            Err(e) => super::on_error(conn, e)
        },
        None => warn!("Unable to retrieve socket from map")
    }
}

fn handle_write_event(e: &libc::epoll_event) {
    let fd = e.u64 as RawFd;
    match map_get(fd) {
        Some(conn) => match socket::send(fd) {
            Ok((sent, rearm_rw)) => {
                debug!("Sent {} bytes to {:?}", sent, conn);
                if rearm_rw {
                    epoll_rearm_rw(conn.socket);
                } else {
                    epoll_rearm_r(conn);
                }
            }
            Err(e) => super::on_error(conn, e)
        },
        None => warn!("Unable to retrieve socket from map")
    }
}

fn epfd() -> RawFd { unsafe { _epfd } }

fn epoll_rearm_r(c: Connection) {
    let mut event = epoll_event(&c);
    set_events_r(&mut event);
    let _ = epoll_mod(event).map_err(|e| {
        error!("During rearm_r {}", e);
    });
}

fn epoll_rearm_rw(fd: RawFd) {
    match map_get(fd) {
        Some(c) => {
            let mut event = epoll_event(&c);
            set_events_rw(&mut event);
            let _ = epoll_mod(event).map_err(|e| {
                error!("During rearm_rw {}", e);
            });
        }
        None => warn!("Unable to find connection in map during re-arm")
    }
}

fn epoll_event(conn: &Connection) -> libc::epoll_event {
    libc::epoll_event { events: 0, u64: conn.socket as u64 }
}

fn epoll_add(mut e: libc::epoll_event) -> io::Result<()> {
    epoll::ctl(epfd(), EPOLL_CTL_ADD, e.u64 as RawFd, &mut e)
}

fn epoll_del(mut e: libc::epoll_event) -> io::Result<()> {
    epoll::ctl(epfd(), EPOLL_CTL_DEL, e.u64 as RawFd, &mut e)
}

fn epoll_mod(mut e: libc::epoll_event) -> io::Result<()> {
    epoll::ctl(epfd(), EPOLL_CTL_MOD, e.u64 as RawFd, &mut e)
}

fn set_events_r(e: &mut libc::epoll_event) {
    let r = EPOLLET | EPOLLONESHOT | EPOLLIN | EPOLLRDHUP;
    e.events = r.bits();
}

fn set_events_rw(e: &mut libc::epoll_event) {
    let rw = EPOLLET | EPOLLONESHOT | EPOLLIN | EPOLLOUT | EPOLLRDHUP;
    e.events = rw.bits();
}

fn close_event(e: u32) -> bool {
    e & (EPOLLERR | EPOLLHUP | EPOLLRDHUP).bits() > 0
}

fn read_event(e: u32) -> bool {
    e & EPOLLIN.bits() > 0
}

fn write_event(e: u32) -> bool {
    e & EPOLLOUT.bits() > 0
}

fn map_add(c: Connection) {
    let mut map = (*CONN_MAP).lock();
    map.insert(c.socket, c);
}

fn map_del(c: Connection) {
    let mut map = (*CONN_MAP).lock();
    map.remove(&c.socket);
}

fn map_get(fd: RawFd) -> Option<Connection> {
    let map = (*CONN_MAP).lock();
    match map.get(&fd) {
        Some(c) => Some(*c),
        None => None
    }
}

fn socket_error(e: u32) -> bool {
    e & EPOLLERR.bits() > 0
}
