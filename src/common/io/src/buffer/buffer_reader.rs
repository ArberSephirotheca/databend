// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::cmp;
use std::fmt;
use std::io;
use std::io::BorrowedBuf;
use std::io::BorrowedCursor;
use std::io::ErrorKind;
use std::io::IoSliceMut;
use std::io::Read;
use std::io::Result;
use std::mem::MaybeUninit;

use crate::buffer::BufferRead;

/// The `BufferReader<R>` portable rust's std::io::BuffReader
pub struct BufferReader<R> {
    inner: R,
    buf: Box<[MaybeUninit<u8>]>,
    pos: usize,
    cap: usize,
    init: usize,
}

impl<R: Read> BufferReader<R> {
    pub fn new(inner: R) -> BufferReader<R> {
        BufferReader::with_capacity(8 * 1024, inner)
    }

    pub fn with_capacity(capacity: usize, inner: R) -> BufferReader<R> {
        let buf = Box::new_uninit_slice(capacity);
        BufferReader {
            inner,
            buf,
            pos: 0,
            cap: 0,
            init: 0,
        }
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    pub fn buffer(&self) -> &[u8] {
        unsafe { MaybeUninit::slice_assume_init_ref(&self.buf[self.pos..self.cap]) }
    }

    pub fn buffer_mut(&mut self) -> &[u8] {
        unsafe { MaybeUninit::slice_assume_init_mut(&mut self.buf[self.pos..self.cap]) }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Invalidates all data in the internal buffer.
    #[inline]
    fn discard_buffer(&mut self) {
        self.pos = 0;
        self.cap = 0;
    }
}

impl<R: Read> std::io::Read for BufferReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If we don't have any buffered data and we're doing a massive read
        // (larger than our internal buffer), bypass our internal buffer
        // entirely.
        if self.pos == self.cap && buf.len() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read(buf);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read(buf)?
        };
        self.consume(nread);
        Ok(nread)
    }

    fn read_buf(&mut self, mut cursor: BorrowedCursor<'_>) -> io::Result<()> {
        // If we don't have any buffered data and we're doing a massive read
        // (larger than our internal buffer), bypass our internal buffer
        // entirely.
        if self.pos == self.cap && cursor.capacity() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read_buf(cursor.reborrow());
        }

        let prev = cursor.written();

        let mut rem = self.fill_buf()?;
        rem.read_buf(cursor.reborrow())?;

        self.consume(cursor.written() - prev); //slice impl of read_buf known to never unfill buf

        Ok(())
    }

    // Small read_exacts from a BufReader are extremely common when used with a deserializer.
    // The default implementation calls read in a loop, which results in surprisingly poor code
    // generation for the common path where the buffer has enough bytes to fill the passed-in
    // buffer.
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if self.buffer().len() >= buf.len() {
            buf.copy_from_slice(&self.buffer()[..buf.len()]);
            self.consume(buf.len());
            return Ok(());
        }

        default_read_exact(self, buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        let total_len = bufs.iter().map(|b| b.len()).sum::<usize>();
        if self.pos == self.cap && total_len >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read_vectored(bufs);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read_vectored(bufs)?
        };
        self.consume(nread);
        Ok(nread)
    }

    fn is_read_vectored(&self) -> bool {
        self.inner.is_read_vectored()
    }

    // The inner reader might have an optimized `read_to_end`. Drain our buffer and then
    // delegate to the inner implementation.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let nread = self.cap - self.pos;
        buf.extend_from_slice(self.buffer());
        self.discard_buffer();
        Ok(nread + self.inner.read_to_end(buf)?)
    }

    // The inner reader might have an optimized `read_to_end`. Drain our buffer and then
    // delegate to the inner implementation.
    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        let mut bytes = Vec::new();
        self.read_to_end(&mut bytes)?;
        let string = std::str::from_utf8(&bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )
        })?;
        *buf += string;
        Ok(string.len())
    }
}

impl<R: Read> BufferRead for BufferReader<R> {
    fn working_buf(&self) -> &[u8] {
        self.buffer()
    }

    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.cap {
            debug_assert!(self.pos == self.cap);
            let mut buf = BorrowedBuf::from(&mut (*self.buf));

            // SAFETY: `self.init` is either 0 or set to `readbuf.initialized_len()`
            // from the last time this function was called
            unsafe {
                buf.set_init(self.init);
            }

            self.inner.read_buf(buf.unfilled())?;

            self.cap = buf.len();
            self.init = buf.init_len();

            self.pos = 0;
        }
        Ok(self.buffer())
    }

    fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.cap);
    }
}

impl<R> fmt::Debug for BufferReader<R>
where R: fmt::Debug
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BufferReader")
            .field("reader", &self.inner)
            .field(
                "buffer",
                &format_args!("{}/{}", self.cap - self.pos, self.buf.len()),
            )
            .finish()
    }
}

pub(crate) fn default_read_exact<R: Read + ?Sized>(this: &mut R, mut buf: &mut [u8]) -> Result<()> {
    while !buf.is_empty() {
        match this.read(buf) {
            Ok(0) => break,
            Ok(n) => {
                let tmp = buf;
                buf = &mut tmp[n..];
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if !buf.is_empty() {
        Err(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "failed to fill whole buffer",
        ))
    } else {
        Ok(())
    }
}
