# altf [<img src="https://travis-ci.org/nathansizemore/altf.svg">][badge-url]

Asynchronous Linux TCP/IP Framework

[Documentation][docs-url]

---

altf is a framework for getting a scalable, read I/O event driven socket server
up and running quickly. It uses an event loop backed by [epoll][epoll-url] with
sockets set in Edge Triggered mode. It also handles all of the tedious buffered
I/O that comes when using epoll in Edge Triggered mode..

## Usage

``` rust
extern crate alnio;


use std::io;

use alnio::Connection;


fn main() {
    // Register callbacks
    alnio::register_on_connect(on_new_connection);
    alnio::register_on_error(on_error);
    alnio::register_on_recv(on_data_available);

    // Begining listening on every interface
    alnio::start("0.0.0.0:1337");
}

fn on_new_connection(conn: &Connection) {
    // Handle new connection house keeping...
}

fn on_data_available(conn: &Connection) {
    // Take a specific amount from receive buffer
    let mut buf = [0u8; 512];
    let num_recv = conn.recv(&mut buf).unwrap();
    println!("Received: {:?}", &buf[0..num_recv]);

    // - OR -

    // Take it all
    let len = conn.bytes_avail().unwrap();
    let mut buf = Vec::<u8>::with_capacity(len);
    unsafe { buf.set_len(len) };
    let num_recv = conn.recv(&mut buf).unwrap();
    unsafe { buf.set_len(num_recv) };
    println!("Received: {:?}", buf);
}

fn on_error(conn: &Connection, err: io::Error) {
    let _ = conn.shutdown();
}
```

---

### Author

Nathan Sizemore, nathanrsizemore@gmail.com

### License

altf is available under the MPL-2.0 license. See the LICENSE file for more info.


[badge-url]: https://travis-ci.org/nathansizemore/altf
[docs-url]: https://docs.rs/epoll
[epoll-url]: http://man7.org/linux/man-pages/man7/epoll.7.html
