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

    let addr = ("127.0.0.1", 4000);
    let server = Echo.start(&addr).unwrap();
    let client = UdpClient::connect(addr).unwrap();

    for i in 0..10 {
        let s = format!("Hello World! id={}", i);
        let data = client.call_service(s.as_bytes()).unwrap();
        println!("recv = {:?}", str::from_utf8(&data).unwrap());
    }

    unsafe { server.coroutine().cancel() };
    server.join().ok();
}