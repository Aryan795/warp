//! Tests for the PTY [`EventLoop`], focused on the post-child-exit drain that
//! prevents a final burst of output (e.g. the tail of a large table) from being
//! dropped when the child process exits — the defect behind APP-5099.

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mio::{Interest, Poll, Token};
use parking_lot::FairMutex;

use super::*;
use crate::terminal::SizeInfo;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::model::TerminalModel;
use crate::terminal::writeable_pty::Message;

/// A [`Read`]er that hands out an in-memory buffer in fixed-size chunks and
/// reports EOF once drained, mimicking a PTY leader that still has a burst of
/// the child's final output buffered. It records the total number of bytes the
/// event loop actually read so a test can assert nothing was left behind.
struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
    total_read: Arc<AtomicUsize>,
}

impl Read for ChunkedReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.data.len() {
            // Everything buffered has been consumed: the PTY is fully drained.
            return Ok(0);
        }
        let remaining = self.data.len() - self.pos;
        let n = remaining.min(self.chunk).min(out.len());
        out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        self.total_read.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }
}

/// A minimal [`EventedPty`] whose reader replays a fixed buffer. Only the pieces
/// exercised by `pty_read` / `drain_pty_after_exit` do anything meaningful; the
/// mio registration hooks and writer are inert.
struct MockPty {
    reader: ChunkedReader,
    writer: io::Sink,
    exited_reported: bool,
}

impl MockPty {
    fn new(data: Vec<u8>, chunk: usize, total_read: Arc<AtomicUsize>) -> Self {
        MockPty {
            reader: ChunkedReader {
                data,
                pos: 0,
                chunk,
                total_read,
            },
            writer: io::sink(),
            exited_reported: false,
        }
    }
}

impl EventedReadWrite for MockPty {
    type Reader = ChunkedReader;
    type Writer = io::Sink;

    fn register(&mut self, _: &Poll, _: Interest) -> io::Result<()> {
        Ok(())
    }

    fn reregister(&mut self, _: &Poll, _: Interest) -> io::Result<()> {
        Ok(())
    }

    fn deregister(&mut self, _: &Poll) -> io::Result<()> {
        Ok(())
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn read_token(&self) -> Token {
        PTY_TOKEN
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }

    fn write_token(&self) -> Token {
        PTY_TOKEN
    }
}

impl EventedPty for MockPty {
    fn child_event_token(&self) -> Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        if self.exited_reported {
            None
        } else {
            self.exited_reported = true;
            Some(ChildEvent::Exited)
        }
    }

    fn on_resize(&mut self, _: &SizeInfo) {}

    fn kill(self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Builds a byte buffer that stands in for a large, fast burst of table output.
/// It is deliberately larger than [`MAX_LOCKED_READ`] so that draining it
/// requires more than one `pty_read` pass.
fn large_output_burst() -> Vec<u8> {
    let mut out = String::new();
    let mut row = 0;
    while out.len() <= MAX_LOCKED_READ * 4 {
        out.push_str(&format!(
            "| col-a-{row:05} | col-b-{row:05} | col-c-{row:05} |\n"
        ));
        row += 1;
    }
    out.into_bytes()
}

fn make_event_loop(pty: MockPty) -> EventLoop<MockPty> {
    let terminal = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let listener = ChannelEventListener::new_for_test();
    let (_tx, rx) = mio_channel::channel::<Message>();
    EventLoop::new(terminal, listener, pty, rx)
}

/// Regression test for APP-5099: after the child exits, the event loop must
/// drain and process **all** output still buffered in the PTY, so the tail of a
/// large/fast burst (e.g. the bottom of a table) is not dropped.
#[test]
fn drain_pty_after_exit_reads_all_buffered_output() {
    let data = large_output_burst();
    let total = data.len();
    let total_read = Arc::new(AtomicUsize::new(0));

    let pty = MockPty::new(data, 4096, total_read.clone());
    let mut event_loop = make_event_loop(pty);

    let mut state = State::default();
    let mut buf = vec![0u8; READ_BUFFER_SIZE];

    event_loop.drain_pty_after_exit(&mut state, &mut buf);

    assert_eq!(
        total_read.load(Ordering::SeqCst),
        total,
        "the entire buffered PTY output must be read on child exit; leaving any \
         bytes unread is exactly the mid-table truncation this fix prevents",
    );
}

/// Guards the assumption the fix relies on: a single `pty_read` intentionally
/// stops after [`MAX_LOCKED_READ`] bytes to yield the terminal lock, so it does
/// **not** drain a burst larger than that on its own. This is why the child-exit
/// path must loop via `drain_pty_after_exit` — without it, tearing down the loop
/// right after observing the exit would drop the remainder.
#[test]
fn single_pty_read_stops_before_draining_large_burst() {
    let data = large_output_burst();
    let total = data.len();
    let total_read = Arc::new(AtomicUsize::new(0));

    let pty = MockPty::new(data, 4096, total_read.clone());
    let mut event_loop = make_event_loop(pty);

    let mut state = State::default();
    let mut buf = vec![0u8; READ_BUFFER_SIZE];
    let mut can_read = true;
    event_loop
        .pty_read(&mut state, &mut buf, &mut can_read)
        .expect("read from the mock PTY succeeds");

    assert!(
        total_read.load(Ordering::SeqCst) < total,
        "a single pty_read should stop early (after MAX_LOCKED_READ) and leave \
         output buffered, demonstrating why the drain loop is required",
    );
}
