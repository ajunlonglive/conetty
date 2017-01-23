use std::io;
use std::sync::Arc;
use std::net::ToSocketAddrs;
use coroutine;
use errors::WireError;
use comanaged::Manager;
use coroutine::sync::Mutex;
use io::{Request, Response};
use bincode::serde as encode;
use bincode::SizeLimit::Infinite;
use coroutine::net::{UdpSocket, TcpListener};

/// must impl this trait for your server
pub trait Service: Send + Sync + Sized + 'static {
    /// the service that would run in a coroutine
    /// the real request should be deserialized from the input
    /// the real response should be serialized into the raw vec
    /// if deserialize/serialize error happened, return an Err(WireError)
    /// application error should be encap in the raw vec
    /// here passed in a self ref to impl stateful service if you want
    fn service(&self, request: &[u8]) -> Result<Vec<u8>, WireError>;
}

/// Provides a function for starting the service.
pub trait UdpServer: Service {
    /// Spawns the service, binding to the given address
    /// return a coroutine that you can cancel it when need to stop the service
    fn start<L: ToSocketAddrs>(self, addr: L) -> io::Result<coroutine::JoinHandle<()>> {
        let sock = UdpSocket::bind(addr)?; // the write half
        let sock1 = sock.try_clone()?; // the read half
        coroutine::Builder::new().name("UdpServer".to_owned()).spawn(move || {
            let server = Arc::new(self);
            let mut buf = vec![0u8; 1024];
            // the write half need to be protected by mutex
            // for that coroutine io obj can't shared safely
            let sock = Arc::new(Mutex::new(sock));
            loop {
                // each udp packet should be less than 1024 bytes
                let (len, addr) = t!(sock1.recv_from(&mut buf));
                info!("recv_from: len={:?} addr={:?}", len, addr);

                // if we failed to deserialize the request frame, just continue
                let req: Request = t!(encode::deserialize(&buf));
                let sock = sock.clone();
                let server = server.clone();
                // let mutex = mutex.clone();
                coroutine::spawn(move || {
                    let rsp = Response {
                        id: req.id,
                        data: server.service(&req.data),
                    };

                    // serialize the result, ignore the error
                    let data = match encode::serialize(&rsp, Infinite) {
                        Ok(data) => data,
                        Err(err) => return error!("udp server serialize failed, err={:?}", err),
                    };

                    info!("send_to: len={:?} addr={:?}", data.len(), addr);

                    // send the result back to client
                    // udp no need to proect by a mutex, each send would be one frame
                    let s = sock.lock().unwrap();
                    match s.send_to(&data, addr) {
                        Ok(_) => {}
                        Err(err) => return error!("udp send_to failed, err={:?}", err),
                    }
                });
            }
        })
    }
}

/// Provides a function for starting the tcp service.
pub trait TcpServer: Service {
    /// Spawns the service, binding to the given address
    /// return a coroutine that you can cancel it when need to stop the service
    fn start<L: ToSocketAddrs>(self, addr: L) -> io::Result<coroutine::JoinHandle<()>> {
        let listener = TcpListener::bind(addr)?;
        coroutine::Builder::new().name("UdpServer".to_owned()).spawn(move || {
            let server = Arc::new(self);
            let manager = Manager::new();
            for stream in listener.incoming() {
                let stream = t!(stream);
                let server = server.clone();
                manager.add(move |_| {
                    // the read half of the stream
                    let mut r_stream = stream.try_clone().expect("failed to clone stream");
                    // the write half need to be protected by mutex
                    // for that coroutine io obj can't shared safely
                    let w_stream = Arc::new(Mutex::new(stream));
                    loop {
                        let req: Request = t!(encode::deserialize_from(&mut r_stream, Infinite));
                        info!("get request: id={:?}", req.id);
                        let w_stream = w_stream.clone();
                        let server = server.clone();
                        coroutine::spawn(move || {
                            let rsp = Response {
                                id: req.id,
                                data: server.service(&req.data),
                            };

                            info!("send rsp: id={}", req.id);
                            // send the result back to client
                            let mut w = w_stream.lock().unwrap();
                            // serialize the result, ignore the error
                            match encode::serialize_into(&mut *w, &rsp, Infinite) {
                                Ok(_) => {}
                                Err(err) => return error!("tcp serialize failed, err={:?}", err),
                            };
                        });
                    }
                });
            }
        })
    }
}

impl<T: Service> UdpServer for T {}
impl<T: Service> TcpServer for T {}
