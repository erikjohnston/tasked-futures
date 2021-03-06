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


struct FnWrapper<F, T, R, E> where F: FnMut(&mut T) -> Poll<R, E> {
    func: F,
    _phantom_data: PhantomData<(T, R, E)>,
}

impl<F, T, R, E> FnWrapper<F, T, R, E> where F: FnMut(&mut T) -> Poll<R, E> {
    pub fn new(f: F) -> FnWrapper<F, T, R, E> {
        FnWrapper {
            func: f,
            _phantom_data: PhantomData,
        }
    }
}

impl<F, T, R, E> TaskedFuture for FnWrapper<F, T, R, E> where F: FnMut(&mut T) -> Poll<R, E> {
    type Item = R;
    type Error = E;
    type Task = T;

    fn poll(&mut self, task: &mut Self::Task) -> Poll<Self::Item, Self::Error> {
        (self.func)(task)
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


pub struct TaskExecutorQueue<T, E> {
    queue: Vec<BoxedExecutorFuture<T, E>>,
    stopped: bool,
}

impl<T, E> TaskExecutorQueue<T, E> {
    pub fn new() -> TaskExecutorQueue<T, E> {
        TaskExecutorQueue {
            queue: Vec::new(),
            stopped: false,
        }
    }

    pub fn stop(&mut self) {
        self.queue.clear();
        self.stopped = true;
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
}

impl<T, E> Default for TaskExecutorQueue<T, E> {
    fn default() -> TaskExecutorQueue<T, E> {
        TaskExecutorQueue::new()
    }
}

pub trait TaskExecutor: Sized + 'static {
    type Error;

    fn task_executor_mut(&mut self) -> &mut TaskExecutorQueue<Self, Self::Error>;

    fn stop(&mut self) {
        self.task_executor_mut().stop();
    }

    fn spawn<F>(&mut self, future: F) where F: IntoTaskedFuture<Item=(), Error=Self::Error, Task=Self> + 'static {
        if !self.task_executor_mut().is_stopped() {
            self.task_executor_mut().queue.push(Box::new(future.into_tasked_future()));
        }
    }

    fn spawn_fn<F>(&mut self, func: F) where F: FnMut(&mut Self) -> Poll<(), Self::Error> + 'static {
        if !self.task_executor_mut().is_stopped() {
            self.task_executor_mut().queue.push(Box::new(FnWrapper::new(func)));
        }
    }

    fn poll(&mut self) -> Poll<(), Self::Error> {
        loop {
            if self.task_executor_mut().is_stopped() {
                return Ok(Async::Ready(()));
            }

            let len = self.task_executor_mut().queue.len();
            let queue = mem::replace(&mut self.task_executor_mut().queue, Vec::with_capacity(len));

            let mut to_append = Vec::with_capacity(len);
            for mut future in queue {
                if let Async::NotReady = try!(future.poll(self)) {
                    to_append.push(future);
                }
            }

            let new_futures = !self.task_executor_mut().queue.is_empty();

            self.task_executor_mut().queue.append(&mut to_append);

            if !new_futures {
                break;
            }
        }

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

    #[test]
    fn fnmut() {
        let mut task = TestTask::default();
        task.spawn_fn(|task| {
            task.data.set(task.data.get() + 1);
            Ok(Async::NotReady)
        });

        let data = task.data.clone();
        assert_eq!(Ok(Async::NotReady), task.poll());
        assert_eq!(data.get(), 1);
        assert_eq!(Ok(Async::NotReady), task.poll());
        assert_eq!(data.get(), 2);
    }
}
