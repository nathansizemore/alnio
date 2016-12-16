// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


use std::mem;

use parking_lot::Mutex;


pub struct SocketBuffer {
    buf: Mutex<Vec<u8>>
}

impl SocketBuffer {
    pub fn new() -> SocketBuffer { SocketBuffer { buf: Mutex::new(Vec::new()) } }

    pub fn len(&self) -> usize {
        let v = self.buf.lock();
        v.len()
    }

    pub fn push(&self, buf: &[u8]) {
        let mut v = self.buf.lock();
        v.extend_from_slice(buf);
    }

    pub fn push_front(&self, buf: &[u8]) {
        let mut v = self.buf.lock();

        let total_len = buf.len() + (*v).len();
        let mut nv = Vec::<u8>::with_capacity(total_len);
        nv.extend_from_slice(buf);
        nv.extend_from_slice(&(*v)[..]);
        mem::swap(&mut *v, &mut nv);
    }

    pub fn take(&self, max: usize) -> Vec<u8> {
        let mut v = self.buf.lock();

        let len = if v.len() < max { v.len() } else { max };
        let mut ret_buf = Vec::<u8>::with_capacity(len);
        ret_buf.extend_from_slice(&v[0..len]);

        unsafe {
            let remaining_len = v.len() - len;
            for x in 0..remaining_len {
                let new = *v.get_unchecked(x + len);
                let old = v.get_unchecked_mut(x);
                *old = new;
            }

            v.set_len(remaining_len);
        }

        ret_buf
    }

    pub fn take_all(&self) -> Vec<u8> {
        let mut v = self.buf.lock();
        mem::replace(&mut *v, Vec::new())
    }
}
