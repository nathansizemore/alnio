// Copyright 2016 Nathan Sizemore <nathanrsizemore@gmail.com>
//
// This Source Code Form is subject to the terms of the
// Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed
// with this file, you can obtain one at http://mozilla.org/MPL/2.0/.


use std::mem;

use parking_lot::Mutex;


pub struct Buffer {
    mutex: Mutex<Vec<u8>>
}

impl Buffer {
    /// Creates a new Buffer.
    pub fn new() -> Buffer { Buffer { mutex: Mutex::new(Vec::new()) } }

    /// Inserts `buf` at the end of this Buffer, allocating if needed.
    pub fn append(&self, buf: &[u8]) {
        let mut v = self.mutex.lock();
        v.extend_from_slice(buf);
    }

    /// Returns the number of elements in this buffer.
    pub fn len(&self) -> usize {
        let v = self.mutex.lock();
        v.len()
    }

    /// Inserts `buf` at the beginning of this Buffer, allocating if needed
    /// and shifting all elements to the right.
    pub fn prepend(&self, buf: &[u8]) {
        let mut v = self.mutex.lock();

        let new_len = buf.len() + (*v).len();
        let mut nv = Vec::<u8>::with_capacity(new_len);
        nv.extend_from_slice(buf);
        nv.extend_from_slice(&(*v)[..]);
        mem::swap(&mut *v, &mut nv);
    }

    /// Extracts up to elements `[0..len]` from this Buffer.
    pub fn extract(&self, len: usize) -> Vec<u8> {
        let mut v = self.mutex.lock();

        let len = if v.len() < len { v.len() } else { len };
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
}

unsafe impl Send for Buffer { }
unsafe impl Sync for Buffer { }
