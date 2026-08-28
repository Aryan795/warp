use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use mio::{Interest, Token};
use parking_lot::FairMutex;

use super::{ControlClientEventLoop, SharedControlState, TmuxControlSender};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::tmux::protocol::kill_session_command;
use crate::terminal::writeable_pty::Message;
use crate::terminal::writeable_pty::pty_controller::EventLoopSender as _;
use crate::terminal::{SizeInfo, TerminalModel};

const IO_TOKEN: Token = Token(1);
const CHILD_TOKEN: Token = Token(2);

struct FakeReader {
    error: Option<io::ErrorKind>,
}

impl Read for FakeReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.error {
            return Err(io::Error::new(kind, "fake read error"));
        }
        Err(io::Error::new(io::ErrorKind::WouldBlock, "no fake bytes"))
    }
}

struct FakeWriter {
    written: Arc<Mutex<Vec<u8>>>,
    error: Option<io::ErrorKind>,
}

impl Write for FakeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.error {
            return Err(io::Error::new(kind, "fake write error"));
        }
        self.written
            .lock()
            .expect("writer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FakePty {
    stream: mio::net::UnixStream,
    reader: FakeReader,
    writer: FakeWriter,
}

fn fake_pty(
    reader_error: Option<io::ErrorKind>,
    writer_error: Option<io::ErrorKind>,
) -> (FakePty, mio::net::UnixStream, Arc<Mutex<Vec<u8>>>) {
    let (stream, peer) = mio::net::UnixStream::pair().expect("unix stream pair");
    let written = Arc::new(Mutex::new(Vec::new()));
    (
        FakePty {
            stream,
            reader: FakeReader {
                error: reader_error,
            },
            writer: FakeWriter {
                written: written.clone(),
                error: writer_error,
            },
        },
        peer,
        written,
    )
}

impl EventedReadWrite for FakePty {
    type Reader = FakeReader;
    type Writer = FakeWriter;

    fn register(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .register(&mut self.stream, IO_TOKEN, interest)
    }

    fn reregister(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .reregister(&mut self.stream, IO_TOKEN, interest)
    }

    fn deregister(&mut self, poll: &mio::Poll) -> io::Result<()> {
        poll.registry().deregister(&mut self.stream)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn read_token(&self) -> Token {
        IO_TOKEN
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }

    fn write_token(&self) -> Token {
        IO_TOKEN
    }
}

impl EventedPty for FakePty {
    fn child_event_token(&self) -> Token {
        CHILD_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }

    fn on_resize(&mut self, _size: &crate::terminal::SizeInfo) {}

    fn kill(self) -> Result<()> {
        Ok(())
    }
}

struct Harness {
    handle: JoinHandle<()>,
    sender: TmuxControlSender,
    model: Arc<FairMutex<TerminalModel>>,
    wakeups_rx: async_channel::Receiver<()>,
    written: Arc<Mutex<Vec<u8>>>,
    peer: Option<mio::net::UnixStream>,
}

fn start_loop(reader_error: Option<io::ErrorKind>, writer_error: Option<io::ErrorKind>) -> Harness {
    let (pty, peer, written) = fake_pty(reader_error, writer_error);
    let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
    let listener = ChannelEventListener::builder_for_test::<crate::terminal::event::Event>()
        .with_wakeups_tx(wakeups_tx)
        .build();
    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let (tx, rx) = mio_channel::channel();
    let shared = Arc::new(SharedControlState::new());
    let sender = TmuxControlSender::new(tx, shared.clone());
    let event_loop = ControlClientEventLoop::new(model.clone(), listener, pty, rx, shared, None);
    Harness {
        handle: event_loop.spawn(),
        sender,
        model,
        wakeups_rx,
        written,
        peer: Some(peer),
    }
}

fn join_loop(handle: JoinHandle<()>) {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("tmux control-mode event loop did not stop");
}

#[test]
fn shutdown_writes_kill_session_before_stopping() {
    let harness = start_loop(None, None);
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written = harness.written.lock().expect("written lock").clone();
    assert_eq!(written, kill_session_command().as_bytes());
    assert!(!harness.model.lock().is_read_only());
}

#[test]
fn read_error_exits_terminal_and_wakes_view() {
    let mut harness = start_loop(Some(io::ErrorKind::ConnectionReset), None);
    let mut peer = harness.peer.take().expect("peer");
    peer.write_all(&[1]).expect("wake readable");
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}

#[test]
fn write_error_exits_terminal_and_wakes_view() {
    let harness = start_loop(None, Some(io::ErrorKind::BrokenPipe));
    harness
        .sender
        .send(Message::Resize(SizeInfo::new_without_font_metrics(24, 80)))
        .expect("send resize");
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}

#[test]
fn closed_pty_exits_terminal_and_wakes_view() {
    let mut harness = start_loop(None, None);
    drop(harness.peer.take());
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}
