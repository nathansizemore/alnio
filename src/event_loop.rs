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

use epoll::{
    self,
    Events,
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
    let e = epoll::Event::new(epoll_events_r(), conn.socket as u64);
    epoll_add(e)
}

pub fn del_conn(conn: Connection) -> io::Result<()> {
    map_del(conn);
    let e = epoll::Event::new(epoll_events_r(), conn.socket as u64);
    epoll_del(e)
}

pub fn needs_write(fd: RawFd) -> io::Result<()> {
    let e = epoll::Event::new(epoll_events_rw(), fd as u64);
    epoll_mod(e)
}

fn event_loop() {
    info!("Starting event loop");

    const WAIT_FOREVER: i32 = -1;
    let mut buf: [epoll::Event; 100] = unsafe { mem::uninitialized() };
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

fn handle_epoll_event(e: &epoll::Event) {
    if close_event(e.events()) {
        handle_close_event(e);
    } else {
        if read_event(e.events()) {
            handle_read_event(e);
        }

        if write_event(e.events()) {
            handle_write_event(e);
        }
    }
}

fn handle_close_event(e: &epoll::Event) {
    let fd = e.data() as RawFd;

    let err = {
        if socket_error(e.events()) {
            match socket::get_last_error(fd) {
                Some(err) => err,
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

fn handle_read_event(e: &epoll::Event) {
    let fd = e.data() as RawFd;
    match map_get(fd) {
        Some(conn) => match socket::recv(fd) {
            Ok(read) => {
                debug!("Recv {} bytes from {:?}", read, conn);
                epoll_rearm_r(fd);
                super::on_recv(conn);
            }
            Err(err) => super::on_error(conn, err)
        },
        None => warn!("Unable to retrieve socket from map")
    }
}

fn handle_write_event(e: &epoll::Event) {
    let fd = e.data() as RawFd;
    match map_get(fd) {
        Some(conn) => match socket::send(fd) {
            Ok((sent, rearm_rw)) => {
                debug!("Sent {} bytes to {:?}", sent, conn);
                if rearm_rw {
                    epoll_rearm_rw(fd);
                } else {
                    epoll_rearm_r(fd);
                }
            }
            Err(err) => super::on_error(conn, err)
        },
        None => warn!("Unable to retrieve socket from map")
    }
}

fn epfd() -> RawFd { unsafe { _epfd } }

fn epoll_rearm_r(fd: RawFd) {
    let e = epoll::Event::new(epoll_events_r(), fd as u64);
    let _ = epoll_mod(e).map_err(|err| {
        error!("During rearm_r {}", err);
    });
}

fn epoll_rearm_rw(fd: RawFd) {
    let e = epoll::Event::new(epoll_events_rw(), fd as u64);
    let _ = epoll_mod(e).map_err(|err| {
        error!("During rearm_rw {}", err);
    });
}

fn epoll_events_r() -> epoll::Events {
    EPOLLET | EPOLLONESHOT | EPOLLIN | EPOLLRDHUP
}

fn epoll_events_rw() -> epoll::Events {
    EPOLLET | EPOLLONESHOT | EPOLLIN | EPOLLOUT | EPOLLRDHUP
}

fn epoll_add(e: epoll::Event) -> io::Result<()> {
    epoll_ctl(EPOLL_CTL_ADD, e)
}

fn epoll_del(e: epoll::Event) -> io::Result<()> {
    epoll_ctl(EPOLL_CTL_DEL, e)
}

fn epoll_mod(e: epoll::Event) -> io::Result<()> {
    epoll_ctl(EPOLL_CTL_MOD, e)
}

fn epoll_ctl(op: epoll::ControlOptions, e: epoll::Event) -> io::Result<()> {
    epoll::ctl(epfd(), op, e.data() as RawFd, e)
}

fn close_event(e: Events) -> bool {
    (e & (EPOLLERR | EPOLLHUP | EPOLLRDHUP)).bits() > 0
}

fn read_event(e: Events) -> bool {
    (e & EPOLLIN).bits() > 0
}

fn write_event(e: Events) -> bool {
    (e & EPOLLOUT).bits() > 0
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

fn socket_error(e: Events) -> bool {
    (e & EPOLLERR).bits() > 0
}
