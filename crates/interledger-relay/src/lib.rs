#![forbid(unsafe_code)]

pub mod app;
mod client;
mod middlewares;
mod routes;
mod services;
#[cfg(test)]
mod testing;

use futures::prelude::*;

pub use self::client::{Client, ClientBuilder};
pub use self::middlewares::{AuthToken};
pub use self::services::relay::{NextHop, Route};

// TODO relay ilp-peer-name, ilp-destination?

pub trait Service: Clone {
    type Future: 'static + Send + Future<
        Item = ilp::Fulfill,
        Error = ilp::Reject,
    >;

    fn call(self, prepare: ilp::Prepare) -> Self::Future;
}
