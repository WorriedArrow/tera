//! runtime-agnostic primitives: they only use wakers, so the same code runs
//! on tokio and on the sim executor. std::sync::Mutex is fine here because no
//! lock is held across an await.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

struct OneshotState<T> {
    value: Option<T>,
    waker: Option<Waker>,
    sender_gone: bool,
}

pub struct OneshotSender<T> {
    st: Arc<Mutex<OneshotState<T>>>,
}

pub struct OneshotReceiver<T> {
    st: Arc<Mutex<OneshotState<T>>>,
}

pub fn oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    let st = Arc::new(Mutex::new(OneshotState {
        value: None,
        waker: None,
        sender_gone: false,
    }));
    (OneshotSender { st: st.clone() }, OneshotReceiver { st })
}

impl<T> OneshotSender<T> {
    pub fn send(self, v: T) {
        let w = {
            let mut g = self.st.lock().unwrap_or_else(|p| p.into_inner());
            g.value = Some(v);
            g.waker.take()
        };
        if let Some(w) = w {
            w.wake();
        }
    }
}

impl<T> Drop for OneshotSender<T> {
    fn drop(&mut self) {
        let w = {
            let mut g = self.st.lock().unwrap_or_else(|p| p.into_inner());
            g.sender_gone = true;
            g.waker.take()
        };
        if let Some(w) = w {
            w.wake();
        }
    }
}

impl<T> Future for OneshotReceiver<T> {
    type Output = Option<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut g = self.st.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(v) = g.value.take() {
            return Poll::Ready(Some(v));
        }
        if g.sender_gone {
            return Poll::Ready(None);
        }
        g.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
