extern crate futures;

use futures::{Async, Future, Poll};

use std::marker::PhantomData;
use std::mem;

mod and_then;
mod chain;
mod map;

pub use and_then::AndThen;
pub use map::TaskedMap;


pub trait TaskedFuture {
    type Item;
    type Error;
    type Task;

    fn poll(&mut self, t: &mut Self::Task) -> Poll<Self::Item, Self::Error>;

    fn map<F, U>(self, f: F) -> TaskedMap<Self, F> where F: FnOnce(Self::Item, &mut Self::Task) -> U, Self: Sized {
        map::new(self, f)
    }

    fn and_then<F, B>(self, f: F) -> AndThen<Self, B, F>
        where F: FnOnce(Self::Item, &mut Self::Task) -> B,
              B: IntoTaskedFuture<Error = Self::Error, Task=Self::Task>,
              Self: Sized,
    {
        and_then::new(self, f)
    }

}


pub trait IntoTaskedFuture {
    /// The future that this type can be converted into.
    type TaskedFuture: TaskedFuture<Item=Self::Item, Error=Self::Error, Task=Self::Task>;

    type Item;
    type Error;
    type Task;

    fn into_tasked_future(self) -> Self::TaskedFuture;
}


impl<T> IntoTaskedFuture for T where T: TaskedFuture {
    type TaskedFuture = Self;

    type Item = <Self as TaskedFuture>::Item;
    type Error = <Self as TaskedFuture>::Error;
    type Task = <Self as TaskedFuture>::Task;

    fn into_tasked_future(self) -> Self::TaskedFuture {
        self
    }
}


pub trait FutureTaskedExt: Future + Sized {
    fn into_tasked<T>(self) -> TaskedFutureAdaptor<Self, T> {
        TaskedFutureAdaptor::new(self)
    }
}


impl<F: Future + Sized> FutureTaskedExt for F {}


pub struct TaskedFutureAdaptor<F, T> {
    future: F,
    _phantom_data: PhantomData<T>,
}

impl<F, T> TaskedFutureAdaptor<F, T> {
    pub fn new(future: F) -> TaskedFutureAdaptor<F, T> {
        TaskedFutureAdaptor {
            future: future,
            _phantom_data: PhantomData,
        }
    }
}

impl<F, I, E, T> TaskedFuture for TaskedFutureAdaptor<F, T> where F: Future<Item=I, Error=E> {
    type Item = I;
    type Error = E;
    type Task = T;

    fn poll(&mut self, _task: &mut Self::Task) -> Poll<I, E> {
        self.future.poll()
    }
}


type BoxedExecutorFuture<T, E> = Box<TaskedFuture<Item=(), Error=E, Task=T>>;

#[derive(Default)]
pub struct TaskExecutorQueue<T, E> {
    queue: Vec<BoxedExecutorFuture<T, E>>,
}

pub trait TaskExecutor: Sized {
    type Error;

    fn task_executor_mut(&mut self) -> &mut TaskExecutorQueue<Self, Self::Error>;

    fn spawn<F>(&mut self, future: F) where F: TaskedFuture<Item=(), Error=Self::Error, Task=Self> + 'static {
        self.task_executor_mut().queue.push(Box::new(future))
    }

    fn poll(&mut self) -> Poll<(), Self::Error> {
        let len = self.task_executor_mut().queue.len();
        let queue = mem::replace(&mut self.task_executor_mut().queue, Vec::with_capacity(len));

        let mut to_append = Vec::with_capacity(len);
        for mut future in queue {
            if let Async::NotReady = try!(future.poll(self)) {
                to_append.push(future);
            }
        }

        self.task_executor_mut().queue.append(&mut to_append);

        if self.task_executor_mut().queue.is_empty() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn into_future(self) -> TaskExecutorFuture<Self> {
        TaskExecutorFuture { inner: self }
    }
}

pub struct TaskExecutorFuture<T> {
    inner: T
}

impl<T, E> Future for TaskExecutorFuture<T> where T: TaskExecutor<Error=E> {
    type Item = ();
    type Error = E;

    fn poll(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use futures::{Async, Future, empty, oneshot, task, lazy};
    use std::sync::Arc;
    use std::cell::Cell;

    struct DummyUnpark;

    impl task::Unpark for DummyUnpark {
        fn unpark(&self) {}
    }

    #[derive(Default)]
    struct TestTask {
        data: Arc<Cell<u8>>,
        executor: TaskExecutorQueue<TestTask, ()>,
    }

    impl TaskExecutor for TestTask {
        type Error = ();

        fn task_executor_mut(&mut self) -> &mut TaskExecutorQueue<Self, Self::Error> {
            &mut self.executor
        }
    }

    #[test]
    fn simple_no_future() {
        let mut task = TestTask::default();
        assert_eq!(Ok(Async::Ready(())), task.poll());
    }

    #[test]
    fn simple_empty_future() {
        let mut task = TestTask::default();

        task.spawn(empty().into_tasked());

        assert_eq!(Ok(Async::NotReady), task.poll());
    }

    #[test]
    fn simeple_oneshot() {
        let (c, p) = oneshot();

        let mut task = TestTask::default();
        task.spawn(p.map_err(|_| ()).into_tasked());

        let mut task = task::spawn(task.into_future());

        assert_eq!(Ok(Async::NotReady), task.poll_future(Arc::new(DummyUnpark)));

        c.complete(());

        assert_eq!(Ok(Async::Ready(())), task.poll_future(Arc::new(DummyUnpark)));
    }

    #[test]
    fn map() {
        let (c, p) = oneshot();

        let mut task = TestTask::default();
        task.spawn(
            p.map_err(|_| ())
                .into_tasked::<TestTask>()
                .map(|res, task| task.data.set(res))
        );

        let data = task.data.clone();
        let mut task = task::spawn(task.into_future());

        assert_eq!(Ok(Async::NotReady), task.poll_future(Arc::new(DummyUnpark)));
        assert_eq!(data.get(), 0);

        c.complete(2u8);

        assert_eq!(Ok(Async::Ready(())), task.poll_future(Arc::new(DummyUnpark)));
        assert_eq!(data.get(), 2);
    }

    #[test]
    fn and_then() {
        let (c, p) = oneshot();

        let mut task = TestTask::default();
        task.spawn(
            p.map_err(|_| ())
                .into_tasked::<TestTask>()
                .and_then(|_, _| {
                    lazy(|| Ok(2u8))
                        .map_err(|_: ()| ())
                        .into_tasked::<TestTask>()
                        .map(|res, task| task.data.set(res))
                })
        );

        let data = task.data.clone();
        let mut task = task::spawn(task.into_future());

        assert_eq!(Ok(Async::NotReady), task.poll_future(Arc::new(DummyUnpark)));
        assert_eq!(data.get(), 0);

        c.complete(2u8);

        assert_eq!(Ok(Async::Ready(())), task.poll_future(Arc::new(DummyUnpark)));
        assert_eq!(data.get(), 2);
    }

    #[test]
    fn spawn() {
        let mut task = TestTask::default();
        task.spawn(
            lazy(|| Ok(()))
                .map_err(|_: ()| ())
                .into_tasked::<TestTask>()
                .map(|_, task| {
                    task.spawn(
                        lazy(|| Ok(2u8))
                            .map_err(|_: ()| ())
                            .into_tasked::<TestTask>()
                            .map(|res, task| task.data.set(res))
                    )
                })
        );

        let data = task.data.clone();
        let mut task = task::spawn(task.into_future());

        assert_eq!(data.get(), 0u8);
        while let Async::NotReady = task.poll_future(Arc::new(DummyUnpark)).unwrap() {}
        assert_eq!(data.get(), 2u8);
    }
}
