extern crate tasked_futures;
extern crate futures;

use futures::Future;
use tasked_futures::{TaskExecutorQueue, TaskExecutor, FutureTaskedExt, TaskedFuture};


#[derive(Default)]
struct Test {
    executor_queue: TaskExecutorQueue<Test, String>,
}

impl TaskExecutor for Test {
    type Error = String;

    fn task_executor_mut(&mut self) -> &mut TaskExecutorQueue<Self, String> {
        &mut self.executor_queue
    }
}

fn main() {
    let mut test = Test::default();

    let future = futures::done(Ok("some result"));

    test.spawn(
        future.into_tasked()
              .map(|lazy_res, _test: &mut Test| {
                  println!("result: {}", lazy_res);
                  // Can now access test and use test.spawn(..) to spawn more things!
              })
    );

    test.into_future().wait().unwrap();
}
