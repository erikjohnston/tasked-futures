use chain::Chain;
use {TaskedFuture, IntoTaskedFuture};

use futures::Poll;


#[must_use = "futures do nothing unless polled"]
pub struct AndThen<A, B, F> where A: TaskedFuture, B: IntoTaskedFuture {
    state: Chain<A, B::TaskedFuture, F>,
}

pub fn new<A, B, F>(future: A, f: F) -> AndThen<A, B, F>
    where A: TaskedFuture,
          B: IntoTaskedFuture<Task=A::Task>,
{
    AndThen {
        state: Chain::new(future, f),
    }
}

impl<A, B, F> TaskedFuture for AndThen<A, B, F>
    where A: TaskedFuture,
          B: IntoTaskedFuture<Error=A::Error, Task=A::Task>,
          F: FnOnce(A::Item, &mut A::Task) -> B,
{
    type Item = B::Item;
    type Error = B::Error;
    type Task = A::Task;

    fn poll(&mut self, task: &mut Self::Task) -> Poll<B::Item, B::Error> {
        self.state.poll(|result, f, task| {
            result.map(|e| {
                Err(f(e, task).into_tasked_future())
            })
        }, task)
    }
}
