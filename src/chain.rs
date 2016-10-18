
use std::mem;

use futures::{Poll, Async};
use TaskedFuture;


pub enum Chain<A, B, C> where A: TaskedFuture {
    First(A, C),
    Second(B),
    Done,
}

impl<A, B, C> Chain<A, B, C>
    where A: TaskedFuture,
          B: TaskedFuture<Task=A::Task>,
{
    pub fn new(a: A, c: C) -> Chain<A, B, C> {
        Chain::First(a, c)
    }

    pub fn poll<F>(&mut self, f: F, task: &mut A::Task) -> Poll<B::Item, B::Error>
        where F: FnOnce(Result<A::Item, A::Error>, C, &mut A::Task)
                        -> Result<Result<B::Item, B>, B::Error>,
    {
        let a_result = match *self {
            Chain::First(ref mut a, _) => {
                match a.poll(task) {
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Ok(Async::Ready(t)) => Ok(t),
                    Err(e) => Err(e),
                }
            }
            Chain::Second(ref mut b) => return b.poll(task),
            Chain::Done => panic!("cannot poll a chained future twice"),
        };
        let data = match mem::replace(self, Chain::Done) {
            Chain::First(_, c) => c,
            _ => panic!(),
        };
        match try!(f(a_result, data, task)) {
            Ok(e) => Ok(Async::Ready(e)),
            Err(mut b) => {
                let ret = b.poll(task);
                *self = Chain::Second(b);
                ret
            }
        }
    }
}
