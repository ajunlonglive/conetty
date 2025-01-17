use std::io::IoSlice;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrayvec::ArrayVec;
use crossbeam::queue::SegQueue;
use may::sync::Mutex;

const MAX_VEC_BUF: usize = 64;

struct VecBufs {
    block: usize,
    pos: usize,
    bufs: ArrayVec<Vec<u8>, MAX_VEC_BUF>,
}

impl VecBufs {
    fn new(bufs: ArrayVec<Vec<u8>, MAX_VEC_BUF>) -> Self {
        VecBufs {
            block: 0,
            pos: 0,
            bufs,
        }
    }

    fn get_io_slice(&self) -> ArrayVec<IoSlice<'_>, MAX_VEC_BUF> {
        let mut ret = ArrayVec::new();
        let first = IoSlice::new(&self.bufs[self.block][self.pos..]);
        ret.push(first);
        for buf in self.bufs.iter().skip(self.block + 1) {
            ret.push(IoSlice::new(buf))
        }
        ret
    }

    fn advance(&mut self, n: usize) {
        let mut left = n;
        for buf in self.bufs[self.block..].iter() {
            let len = buf.len() - self.pos;
            if left >= len {
                left -= len;
                self.block += 1;
                self.pos = 0;
            } else {
                self.pos += left;
                break;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.block == self.bufs.len()
    }

    // write all data from the vecs to the writer
    fn write_all<W: Write>(mut self, writer: &mut W) -> std::io::Result<()> {
        while !self.is_empty() {
            let n = writer.write_vectored(&self.get_io_slice())?;
            self.advance(n);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct QueuedWriter<W: Write> {
    data_count: AtomicUsize,
    data_queue: SegQueue<Vec<u8>>,
    writer: Mutex<W>,
}

impl<W: Write> QueuedWriter<W> {
    pub fn new(writer: W) -> Self {
        QueuedWriter {
            data_count: AtomicUsize::new(0),
            data_queue: SegQueue::new(),
            writer: Mutex::new(writer),
        }
    }

    /// it's safe and efficient to call this API concurrently
    pub fn write(&self, data: Vec<u8>) {
        self.data_queue.push(data);
        // only allow the first writer perform the write operation
        // other concurrent writers would just push the data
        if self.data_count.fetch_add(1, Ordering::AcqRel) == 0 {
            // in any cases this should not block since we have only one writer
            #[allow(clippy::cast_ref_to_mut)]
            let writer = unsafe { &mut *(&self.writer as *const _ as *mut Mutex<W>) };
            let writer = writer.get_mut().unwrap();

            loop {
                let mut total_data = ArrayVec::new();
                while let Some(data) = self.data_queue.pop() {
                    total_data.push(data);
                    if total_data.len() >= MAX_VEC_BUF {
                        break;
                    }
                }

                let cnt = total_data.len();
                let io_bufs = VecBufs::new(total_data);
                if let Err(e) = io_bufs.write_all(&mut *writer) {
                    // FIXME: handle the error
                    error!("QueuedWriter failed, err={}", e);
                }

                // detect if there are more packet need to deal with
                if self.data_count.fetch_sub(cnt, Ordering::AcqRel) == cnt {
                    break;
                }
            }
        }
    }
}
