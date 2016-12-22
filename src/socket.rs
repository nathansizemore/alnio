// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


use std::collections::BTreeMap;
use std::io::{self, Error, ErrorKind};
use std::mem;
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::usize;

use libc;
use parking_lot::Mutex;

use buf::Buffer;


type BufferMap = Mutex<BTreeMap<RawFd, Arc<Buffer>>>;


lazy_static! {
    static ref RX_BUF_MAP: BufferMap = Mutex::new(BTreeMap::new());
    static ref TX_BUF_MAP: BufferMap = Mutex::new(BTreeMap::new());
}


/// Initializes this socket's rx/tx buffers
pub fn init(fd: RawFd) {
    map_add(&RX_BUF_MAP, fd);
    map_add(&TX_BUF_MAP, fd);
}

/// Clears the errno for this specific socket, and returns the errno
/// if an error was present.
pub fn get_last_error(fd: RawFd) -> Option<io::Error> {
    let (r, errno) = unsafe {
        let mut errno: libc::c_int = 0;
        let mut len = mem::size_of::<libc::c_int>() as libc::socklen_t;
        let r = libc::getsockopt(fd,
                                 libc::SOL_SOCKET,
                                 libc::SO_ERROR,
                                 &mut errno as *mut _ as *mut libc::c_void,
                                 &mut len as *mut libc::socklen_t);
        (r, errno)
    };

    if r == -1 {
        let err = Error::last_os_error();
        warn!("{} during getsockopt SO_ERROR for fd {}", err, fd);
        return None;
    }

    if errno == 0 { None } else { Some(Error::from_raw_os_error(errno)) }
}

/// Reads all available data until EAGAIN/EWOULDBLOCK is received, copying
/// all data from kernel space into userspace.
pub fn recv(fd: RawFd) -> io::Result<usize> {
    let maybe_buf = map_get(&RX_BUF_MAP, fd);
    if maybe_buf.is_none() {
        let err = Error::new(ErrorKind::InvalidInput, "Unable to find fd");
        return Err(err);
    }

    let rx_buf = maybe_buf.unwrap();

    const BUF_LEN: usize = 4096;
    let mut buf: [u8; BUF_LEN] = unsafe { mem::uninitialized() };
    let b = buf.as_mut_ptr() as *mut libc::c_void;

    // When the fds are set as EPOLLET mode, we need to read until
    // we receive EAGAIN/EWOULDBLOCK
    let mut total_recvd: usize = 0;
    loop {
        let r = unsafe { libc::recv(fd, b, BUF_LEN, 0) };

        if r == -1 {
            let err = Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock { break; }
            return Err(err);
        } else if r == 0 {
            let err = Error::new(ErrorKind::UnexpectedEof, "EOF");
            return Err(err);
        } else {
            let num_read = r as usize;
            rx_buf.append(&buf[0..num_read]);
            total_recvd += num_read;
        }
    }

    Ok(total_recvd)
}

/// Sends all available data in current userspace buffer.
pub fn send(fd: RawFd) -> io::Result<(usize, bool)> {
    let maybe_buf = map_get(&TX_BUF_MAP, fd);
    if maybe_buf.is_none() {
        let err = Error::new(ErrorKind::InvalidInput, "Unable to find fd");
        return Err(err);
    }

    let sock_buf = maybe_buf.unwrap();
    let buf = sock_buf.extract(usize::MAX);
    let l = buf.len();
    let b = buf.as_ptr() as *const libc::c_void;
    let r = unsafe { libc::send(fd, b, l, 0) };

    // Depending the amount sent/errno code, the caller of this
    // function needs to know if they need to re-arm the fd in epoll
    // with only read OR read and write flags.

    // The only error we care to transform is EAGAIN/EWOULDBLOCK.
    if r == -1 {
        let e = Error::last_os_error();
        if e.kind() == ErrorKind::WouldBlock {
            return Ok((0, true));
        }
        return Err(e);
    }

    // People are dumb, no way to guarantee someone will not call our
    // write pipeline with an empty buffer.
    if r == 0 {
        if buf.len() == 0 { return Ok((0, false)); }
        // This should never happen, but it would do good to check
        // for it anyway.
        return Err(Error::new(ErrorKind::WriteZero, "WriteZero"));
    }

    let sent = r as usize;
    let mut rearm_rw = false;

    if sent < buf.len() {
        // If we sent less than we tried to, this is a result of the
        // internel socket's buffer being full, and we need to push
        // what we didn't send back to the front of the buffer.
        let unsent_buf = &buf[sent..buf.len()];
        sock_buf.prepend(unsent_buf);
        rearm_rw = true;
    }

    Ok((sent, rearm_rw))
}

pub fn shutdown(fd: RawFd) -> io::Result<()> {
    map_del(&RX_BUF_MAP, fd);
    map_del(&TX_BUF_MAP, fd);

    let r = unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
    if r == -1 { Err(Error::last_os_error()) } else { Ok(()) }
}

pub fn add_to_tx_buf(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    let maybe_buf = map_get(&TX_BUF_MAP, fd);
    if maybe_buf.is_none() {
        let err = Error::new(ErrorKind::InvalidInput, "Unable to find fd");
        return Err(err);
    }

    let sock_buf = maybe_buf.unwrap();
    sock_buf.append(buf);

    Ok(buf.len())
}

pub fn peek(fd: RawFd) -> io::Result<usize> {
    match map_get(&RX_BUF_MAP, fd) {
        Some(sock_buf) => Ok(sock_buf.len()),
        None => Err(Error::new(ErrorKind::InvalidInput, "Unable to find fd"))
    }
}

pub fn take(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    match map_get(&RX_BUF_MAP, fd) {
        Some(sock_buf) => {
            let v = sock_buf.extract(buf.len());

            let len = if buf.len() <= v.len() { buf.len() } else { v.len() };
            for x in 0..len {
                buf[x] = unsafe { *v.get_unchecked(x) };
            }

            Ok(len)
        },
        None => Err(Error::new(ErrorKind::InvalidInput, "Unable to find fd"))
    }
}

pub fn close(fd: RawFd) -> io::Result<()> {
    let r = unsafe { libc::close(fd) };
    if r == -1 { Err(Error::last_os_error()) } else { Ok(()) }
}

fn map_add(m: &BufferMap, fd: RawFd) {
    let mut map = (*m).lock();
    map.insert(fd, Arc::new(Buffer::new()));
}

fn map_del(m: &BufferMap, fd: RawFd) {
    let mut map = (*m).lock();
    map.remove(&fd);
}

fn map_get(m: &BufferMap, fd: RawFd) -> Option<Arc<Buffer>> {
    let map = (*m).lock();
    match map.get(&fd) {
        Some(s) => Some((*s).clone()),
        None => None
    }
}
