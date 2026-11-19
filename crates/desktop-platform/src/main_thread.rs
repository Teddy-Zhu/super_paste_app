use super_paste_core::{Error, Result};
use tokio::sync::mpsc;
use std::{future::poll_fn, sync::{Arc, Mutex}};

pub type MainThreadJob = Box<dyn FnOnce() + Send + 'static>;
#[derive(Clone)]
pub struct MainThreadReceiver { receiver: Arc<Mutex<mpsc::Receiver<MainThreadJob>>> }
impl MainThreadReceiver {
    /// The GPUI foreground executor owns the pump while running. After it
    /// returns, main resumes the same queue on the same OS thread until owned
    /// shutdown finishes. The mutex is never held while awaiting or running a job.
    pub async fn recv(&self) -> Option<MainThreadJob> {
        poll_fn(|cx| self.receiver.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).poll_recv(cx)).await
    }
}

/// The desktop host drains the receiver on GPUI's foreground executor.
#[derive(Clone)]
pub struct MainThread { sender: mpsc::Sender<MainThreadJob>, owner: std::thread::ThreadId }
impl MainThread {
    pub fn channel() -> (Self, MainThreadReceiver) {
        let (sender, receiver) = mpsc::channel(64);
        (Self { sender, owner: std::thread::current().id() }, MainThreadReceiver { receiver: Arc::new(Mutex::new(receiver)) })
    }
    pub(crate) fn is_current_thread(&self) -> bool { self.owner == std::thread::current().id() }
    pub fn run_on_main_thread(&self, job: impl FnOnce() + Send + 'static) -> Result<()> {
        self.sender.try_send(Box::new(job)).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => Error::ResourceLimit("平台主线程任务队列已满".into()),
            mpsc::error::TrySendError::Closed(_) => Error::Cancelled,
        })
    }
    /// Owned workers with a fixed concurrency bound may wait for queue capacity.
    /// In particular, native-resource release must not be lost when the UI queue is full.
    pub async fn run_on_main_thread_wait(&self, job: impl FnOnce() + Send + 'static) -> Result<()> {
        self.sender.send(Box::new(job)).await.map_err(|_| Error::Cancelled)
    }

    /// Owned native worker threads may wait for cleanup admission without
    /// losing a native resource when the queue is full. Never call this from
    /// an async worker; the owner itself can perform cleanup immediately.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn run_on_main_thread_blocking(&self, job: impl FnOnce() + Send + 'static) -> Result<()> {
        if self.is_current_thread() { job(); return Ok(()); }
        self.sender.blocking_send(Box::new(job)).map_err(|_| Error::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc as blocking, thread, time::Duration};

    #[test]
    fn native_cleanup_waits_for_capacity_and_runs_on_owner_thread() {
        let (main, receiver) = MainThread::channel();
        let owner = thread::current().id();
        let (inline, inline_done) = blocking::channel();
        main.run_on_main_thread_blocking(move || { inline.send(thread::current().id()).expect("report inline cleanup"); }).expect("inline owner cleanup");
        assert_eq!(inline_done.try_recv().expect("owner cleanup runs before returning"), owner);
        loop {
            match main.run_on_main_thread(|| {}) {
                Ok(()) => {}
                Err(Error::ResourceLimit(_)) => break,
                Err(error) => panic!("fill live main-thread queue: {error}"),
            }
        }
        let (started, ready) = blocking::channel();
        let (submitted, accepted) = blocking::channel();
        let (executed, completed) = blocking::channel();
        let worker_main = main.clone();
        let worker = thread::spawn(move || {
            started.send(()).expect("announce native worker");
            let result = worker_main.run_on_main_thread_blocking(move || {
                executed.send(thread::current().id()).expect("report cleanup thread");
            });
            submitted.send(result).expect("report cleanup admission");
        });
        ready.recv_timeout(Duration::from_secs(5)).expect("native worker started");
        assert!(matches!(accepted.recv_timeout(Duration::from_millis(20)), Err(blocking::RecvTimeoutError::Timeout)), "saturation must retain cleanup instead of rejecting it");
        receiver.receiver.lock().expect("main queue").blocking_recv().expect("queued ordinary job")();
        accepted.recv_timeout(Duration::from_secs(5)).expect("cleanup admitted after capacity returns").expect("cleanup submission");
        assert!(matches!(completed.try_recv(), Err(blocking::TryRecvError::Empty)), "native worker must not execute owner-only cleanup");
        {
            let mut queue = receiver.receiver.lock().expect("main queue");
            while let Ok(job) = queue.try_recv() { job(); }
        }
        assert_eq!(completed.recv_timeout(Duration::from_secs(5)).expect("queued cleanup executed"), owner);
        worker.join().expect("native worker finished");

        drop(receiver);
        let result = thread::spawn(move || main.run_on_main_thread_blocking(|| panic!("closed queue cannot execute cleanup")))
            .join().expect("closed-queue worker finished");
        assert!(matches!(result, Err(Error::Cancelled)));
    }
}
