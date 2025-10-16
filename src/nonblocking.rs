//! Implements a nonblocking write adapter

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::RecvTimeoutError;

enum Argument {
    Flush,
}

pub struct NonBlocking(
    Arc<Mutex<Vec<u8>>>,
    crossbeam_channel::Sender<Argument>,
    crossbeam_channel::Receiver<io::Result<()>>,
);

impl Drop for NonBlocking {
    fn drop(&mut self) {
        _ = self.flush();
    }
}

impl io::Write for NonBlocking {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.1.send(Argument::Flush).expect("must exist");

        self.2.recv().unwrap()
    }
}

pub fn nonblocking<W: io::Write + Send + 'static>(
    mut writer: W,
    interval: Duration,
) -> (NonBlocking, JoinHandle<()>) {
    let (arg_send, arg_recv) = crossbeam_channel::bounded(1);
    let (ret_send, ret_recv) = crossbeam_channel::bounded(1);
    let shared = Arc::new(Mutex::new(Vec::with_capacity(1024 * 1024 * 10)));
    let mut cache = Vec::with_capacity(1024 * 1024 * 10);

    let shared_clone = shared.clone();

    let handle = std::thread::spawn(move || loop {
        let arg = arg_recv.recv_timeout(interval);

        cache = core::mem::replace(&mut shared_clone.lock().unwrap(), cache);

        _ = writer.write_all(&cache);
        let res = writer.flush();

        cache.clear();

        match arg {
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break;
            }
            Ok(Argument::Flush) => {
                _ = ret_send.send(res);
            }
        }
    });

    (NonBlocking(shared, arg_send, ret_recv), handle)
}
