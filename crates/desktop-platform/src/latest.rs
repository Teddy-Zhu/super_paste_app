use crossbeam_channel::{Receiver, Sender, SendError, TrySendError};

/// One pending snapshot. Its owner cancels/joins the producer before dropping the receiver.
/// Native callbacks never block on a busy source/icon consumer.
#[derive(Clone)]
pub(crate) struct LatestSender<T> { sender: Sender<T>, eviction: Receiver<T> }
pub(crate) fn channel<T>() -> (LatestSender<T>, Receiver<T>) {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    (LatestSender { sender, eviction: receiver.clone() }, receiver)
}
impl<T> LatestSender<T> {
    pub(crate) fn send(&self, mut value: T) -> Result<(), SendError<T>> {
        loop {
            match self.sender.try_send(value) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(value)) => return Err(SendError(value)),
                Err(TrySendError::Full(pending)) => { value = pending; let _ = self.eviction.try_recv(); }
            }
        }
    }
}
