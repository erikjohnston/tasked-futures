use TaskedFuture;

use futures::{Poll, Async};


#[must_use = "futures do nothing unless polled"]
pub struct TaskedMap<A, F> where A: TaskedFuture {
    future: A,
    f: Option<F>,
}

pub fn new<A, F>(future: A, f: F) -> TaskedMap<A, F>
    where A: TaskedFuture,
{
    TaskedMap {
        future: future,
        f: Some(f),
    }
}

impl<U, A, F> TaskedFuture for TaskedMap<A, F>
    where A: TaskedFuture,
          F: FnOnce(A::Item, &mut A::Task) -> U,
{
    type Item = U;
    type Error = A::Error;
    type Task = A::Task;

    fn poll(&mut self, task: &mut Self::Task) -> Poll<U, A::Error> {
        let e = match self.future.poll(task) {
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Ok(Async::Ready(e)) => Ok(e),
            Err(e) => Err(e),
        };
        e.map(|res| (self.f.take().expect("cannot poll Map twice"))(res, task))
         .map(Async::Ready)
    }
}
