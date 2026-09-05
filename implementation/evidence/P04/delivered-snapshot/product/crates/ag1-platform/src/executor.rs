use ag1_core::{DomainError, PortFuture};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
    task::{Context, Poll, Waker},
    thread,
};

type Job = Box<dyn FnOnce() + Send>;
/// Fixed worker count and bounded waiting room. Polling a port never blocks on filesystem/Git I/O.
pub struct BlockingPool {
    sender: mpsc::SyncSender<Job>,
}
impl BlockingPool {
    pub fn new(workers: usize, waiting: usize) -> Result<Self, DomainError> {
        if workers == 0 || workers > 16 || waiting == 0 || waiting > 256 {
            return Err(DomainError::InvalidInput("executor limits".into()));
        }
        let (sender, receiver) = mpsc::sync_channel::<Job>(waiting);
        let receiver = Arc::new(Mutex::new(receiver));
        for i in 0..workers {
            let receiver = receiver.clone();
            thread::Builder::new()
                .name(format!("ag1-blocking-{i}"))
                .spawn(move || {
                    loop {
                        let next = receiver.lock().unwrap().recv();
                        match next {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                })
                .map_err(super::io_error)?;
        }
        Ok(Self { sender })
    }
    pub fn submit<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, DomainError> + Send + 'static,
    ) -> PortFuture<'static, T> {
        let state = Arc::new(Mutex::new(State {
            result: None,
            waker: None,
        }));
        let result = state.clone();
        let job = Box::new(move || {
            let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err(DomainError::Io("blocking task panicked".into())));
            let mut state = result.lock().unwrap();
            state.result = Some(value);
            let waker = state.waker.take();
            drop(state);
            if let Some(waker) = waker {
                waker.wake();
            }
        });
        if self.sender.try_send(job).is_err() {
            state.lock().unwrap().result = Some(Err(DomainError::QueueFull));
        }
        Box::pin(Task { state })
    }
}
struct State<T> {
    result: Option<Result<T, DomainError>>,
    waker: Option<Waker>,
}
struct Task<T> {
    state: Arc<Mutex<State<T>>>,
}
impl<T> Future for Task<T> {
    type Output = Result<T, DomainError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
