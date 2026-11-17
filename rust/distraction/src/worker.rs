//! Bounded blocking work, separate from command and audio ownership.
use dt_core::{Error, Result};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::thread::{self, JoinHandle};

pub(crate) struct Worker<Q, A> {
    requests: Option<SyncSender<Q>>,
    pub(crate) replies: Receiver<A>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl<Q: Send + 'static, A: Send + 'static> Worker<Q, A> {
    pub(crate) fn start(capacity: usize, work: impl Fn(Q) -> A + Send + 'static) -> Result<Self> {
        let (requests, receive) = mpsc::sync_channel(capacity);
        let (send, replies) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::Builder::new()
            .name("distraction-worker".into())
            .spawn(move || {
                while let Ok(request) = receive.recv() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let answer = work(request);
                    if stopping.load(Ordering::Acquire) || send.send(answer).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            replies,
            stop,
            thread: Some(thread),
        })
    }
    pub(crate) fn submit(&self, request: Q) -> Result<()> {
        self.requests
            .as_ref()
            .ok_or(Error::Protocol("worker stopped"))?
            .try_send(request)
            .map_err(|_| Error::Protocol("worker queue full or stopped"))
    }
    pub(crate) fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| Error::Protocol("worker failed"))?;
        }
        Ok(())
    }
}
impl<Q, A> Drop for Worker<Q, A> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
