use std::io::{self, BufReader};
use std::time::Duration;

use crate::errors::Error;
use crate::frame::{Frame, ReqBuf};
use crate::queued_writer::QueuedWriter;
use crate::stream_ext::StreamExt;
use crate::Client;

use may::{coroutine, go};
use may_waiter::TokenWaiter;
#[derive(Debug)]
pub struct MultiplexClient<S: StreamExt> {
    // default timeout is 10s
    timeout: Option<Duration>,
    // the connection
    sock: QueuedWriter<S>,
    // the listening coroutine
    listener: Option<coroutine::JoinHandle<()>>,
}

impl<S: StreamExt> Drop for MultiplexClient<S> {
    fn drop(&mut self) {
        if let Some(h) = self.listener.take() {
            unsafe { h.coroutine().cancel() };
            // FIXME: join here when bug fix in thread context in may
            // h.join().ok();
        }
    }
}

impl<S: StreamExt> MultiplexClient<S> {
    /// connect to the server address
    pub fn new(stream: S) -> io::Result<Self> {
        // here we must clone the socket for read
        // we can't share it between coroutines
        let stream1 = stream.try_clone()?;
        let mut r_stream = BufReader::new(stream1);
        let listener = go!(
            coroutine::Builder::new().name("MultiPlexClientListener".to_owned()),
            move || {
                loop {
                    let rsp_frame = match Frame::decode_from(&mut r_stream) {
                        Ok(r) => r,
                        Err(ref e) => {
                            if e.kind() == io::ErrorKind::UnexpectedEof {
                                info!("tcp multiplex_client decode rsp: connection closed");
                            } else {
                                error!("tcp multiplex_client decode rsp: err = {:?}", e);
                            }
                            break;
                        }
                    };
                    info!("receive rsp, id={}", rsp_frame.id);

                    // set the wait req
                    let id = unsafe { may_waiter::ID::from_usize(rsp_frame.id as usize) };
                    TokenWaiter::set_rsp(id, rsp_frame);
                }
            }
        )?;

        Ok(MultiplexClient {
            timeout: None,
            sock: QueuedWriter::new(stream),
            listener: Some(listener),
        })
    }

    /// set the default timeout value
    /// the initial timeout is 10 seconds
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = Some(timeout);
    }
}

impl<S: StreamExt> Client for MultiplexClient<S> {
    fn call_service(&self, req: ReqBuf) -> Result<Frame, Error> {
        let waiter = TokenWaiter::new();
        let id = waiter.id().unwrap();
        info!("request id = {:?}", id);

        // send the request
        let id: usize = id.into();
        let buf = req.finish(id as u64);

        self.sock.write(buf);

        // wait for the rsp
        Ok(waiter.wait_rsp(self.timeout)?)
    }
}
