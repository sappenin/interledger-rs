//#![feature(never_type)]
#![forbid(unsafe_code)]

mod relay;
mod receiver;
mod routes;

use futures::prelude::*;

pub use self::receiver::Receiver;
pub use self::relay::Relay;
pub use self::routes::Route;

pub trait Service<Req> {
    type Future: 'static + Send + Future<
        Item = ilp::Fulfill,
        Error = ilp::Reject,
    >;

    fn call(self, request: Req) -> Self::Future;
}

pub trait Request: Into<ilp::Prepare> {
    fn prepare(&self) -> &ilp::Prepare;
    fn prepare_mut(&mut self) -> &mut ilp::Prepare;
}

/* TODO http tests?
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::prelude::*;
    use hyper::{Request, Server};
    use hyper::service;

    use super::*;

    static URI: &'static str = "http://127.0.0.1:3001/ilp";
    static ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 3001);

    #[test]
    fn test_bad_prepare() {
        with_server({
            let client = hyper::Client::new();
            client
                .request(
                    Request::post(URI)
                        .body(hyper::Body::from("not a packet"))
                        .unwrap(),
                )
                .and_then(|res| {
                    assert_eq!(res.status(), 400);
                    Ok(())
                })
                .map_err(|error| {
                    panic!("unexpected error: {}", error);
                })
        });
    }

    fn with_server<F>(run_test: F)
    where
        F: 'static + Future<Item = ()> + Send,
    {
        let relay = Arc::new(Relay::new(
            b"example.alice".to_vec(),
            vec![
                Route::new(b"".to_vec(), "http://127.0.0.1:3002/ilp".parse::<Uri>().unwrap()),
            ],
        ));
        let server = Server::bind(&ADDR.into())
            .serve(move || {
                let relay = Arc::clone(&relay);
                service::service_fn(move |req| {
                    service::Service::call(&mut &*relay, req)
                })
            });
        hyper::rt::run(
            server
                .with_graceful_shutdown(run_test)
                .map_err(|err| {
                    panic!("server error: {}", err)
                }),
        );
    }
}
*/
