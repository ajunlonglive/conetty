use std::sync::Arc;
use std::net::ToSocketAddrs;
use std::io::{self, Cursor, BufReader, BufWriter, Write};
use Server;
use Response;
use coroutine;
use frame::Frame;
use comanaged::Manager;
use coroutine::sync::Mutex;
use bincode::serde as encode;
use bincode::SizeLimit::Infinite;
use coroutine::net::{UdpSocket, TcpListener};
// use bincode::serde::DeserializeError::IoError;


/// Provides a function for starting the service.
pub trait UdpServer: Server {
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
                let req = t!(Frame::decode_from(&mut Cursor::new(&buf)));
                let sock = sock.clone();
                let server = server.clone();
                // let mutex = mutex.clone();
                coroutine::spawn(move || {
                    let rsp = Response {
                        id: req.id,
                        data: server.service(&req.data).unwrap(),
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
pub trait TcpServer: Server {
    /// Spawns the service, binding to the given address
    /// return a coroutine that you can cancel it when need to stop the service
    fn start<L: ToSocketAddrs>(self, addr: L) -> io::Result<coroutine::JoinHandle<()>> {
        let listener = TcpListener::bind(addr)?;
        coroutine::Builder::new().name("TcpServer".to_owned()).spawn(move || {
            let server = Arc::new(self);
            let manager = Manager::new();
            for stream in listener.incoming() {
                let stream = t!(stream);
                let server = server.clone();
                manager.add(move |_| {
                    let rs = stream.try_clone().expect("failed to clone stream");
                    // the read half of the stream
                    let mut rs = BufReader::new(rs);
                    // the write half need to be protected by mutex
                    // for that coroutine io obj can't shared safely
                    let ws = Arc::new(Mutex::new(BufWriter::new(stream)));

                    loop {
                        let req = match Frame::decode_from(&mut rs) {
                            Ok(r) => r,
                            Err(ref e) => {
                                if e.kind() == io::ErrorKind::UnexpectedEof {
                                    info!("tcp server decode req: connection closed");
                                } else {
                                    error!("tcp server decode req: err = {:?}", e);
                                }
                                break;
                            }
                        };

                        info!("get request: id={:?}", req.id);
                        let w_stream = ws.clone();
                        let server = server.clone();
                        coroutine::spawn(move || {
                            let rsp = Response {
                                id: req.id,
                                data: server.service(&req.data).unwrap(),
                            };
                            drop(req);

                            info!("send rsp: id={}", rsp.id);
                            // send the result back to client
                            let mut w = w_stream.lock().unwrap();
                            // serialize the result, ignore the error
                            match encode::serialize_into(&mut *w, &rsp, Infinite) {
                                Ok(_) => {}
                                Err(err) => return error!("tcp serialize failed, err={:?}", err),
                            };
                            w.flush().unwrap_or_else(|e| error!("flush failed: err={:?}", e));
                        });
                    }
                });
            }
        })
    }
}

impl<T: Server> UdpServer for T {}
impl<T: Server> TcpServer for T {}
