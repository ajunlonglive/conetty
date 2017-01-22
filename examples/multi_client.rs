extern crate conetty;
extern crate coroutine;
extern crate env_logger;

use std::str;
use conetty::{Service, WireError, UdpServer, UdpClient};

struct Echo;

impl Service for Echo {
    fn service(&self, request: &[u8]) -> Result<Vec<u8>, WireError> {
        println!("req = {:?}", request);
        Ok(request.to_vec())
    }
}

fn main() {
    env_logger::init().unwrap();
    coroutine::scheduler_config().set_workers(4).set_io_workers(4);

    let addr = ("127.0.0.1", 4000);
    let server = Echo.start(&addr).unwrap();

    let mut vec = vec![];
    for i in 0..8 {
        let j = coroutine::spawn(move || {
            let client = UdpClient::connect(addr).unwrap();
            for j in 0..10 {
                let s = format!("Hello World! id={}, j={}", i, j);
                match client.call_service(s.as_bytes()) {
                    Ok(data) => println!("recv = {:?}", str::from_utf8(&data).unwrap()),
                    Err(err) => println!("recv err = {:?}", err),
                }
            }
            println!("thread done, id={}", i);
        });
        vec.push(j);
    }

    for (i, j) in vec.into_iter().enumerate() {
        j.join().unwrap();
        println!("wait for {} done", i);
    }

    unsafe { server.coroutine().cancel() };
    server.join().ok();
}