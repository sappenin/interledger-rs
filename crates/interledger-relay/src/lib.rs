//#![feature(never_type)]
#![forbid(unsafe_code)]

mod relay;
mod receiver;
mod routes;

use futures::prelude::*;

pub use self::receiver::Receiver;
pub use self::relay::Relay;
pub use self::routes::Route;

pub trait Service: Clone {
    type Future: 'static + Send + Future<
        Item = ilp::Fulfill,
        Error = ilp::Reject,
    >;

    fn call(self, prepare: ilp::Prepare) -> Self::Future;
}

/*
pub trait Request: Into<ilp::Prepare> {
    fn prepare(&self) -> &ilp::Prepare;
    fn prepare_mut(&mut self) -> &mut ilp::Prepare;
}
*/

/* TODO http tests?
#[cfg(test)]
mod tests {
    use std::sync::Arc;
_
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

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use bytes::{Bytes, BytesMut};
    use futures::prelude::*;
    use hyper::{Uri, StatusCode};
    use hyper::header;
    use lazy_static::lazy_static;
    use tokio::runtime::Runtime;

    use crate::routes::Route;
    use super::*;

    static URI: &'static str = "http://127.0.0.1:3001/ilp";
    static ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 3001);

    lazy_static! {
        static ref PREPARE: ilp::PrepareBuilder<'static> = ilp::PrepareBuilder {
            amount: 123,
            expires_at: SystemTime::now(),
            execution_condition: b"\
                \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
                \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
            ",
            destination: b"test.bob",
            data: b"\
                \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
                \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
            ",
        };

        static ref FULFILL: ilp::FulfillBuilder<'static> = ilp::FulfillBuilder {
            fulfillment: b"\
                \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
                \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
            ",
            data: b"\
                \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
                \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
            ",
        };

        static ref PREPARE_BYTES: Vec<u8> =
            BytesMut::from(PREPARE.build()).to_vec();
        static ref FULFILL_BYTES: Vec<u8> =
            BytesMut::from(FULFILL.build()).to_vec();

        static ref ADDRESS: &'static [u8] = b"example.alice";
        static ref ROUTES: Vec<routes::Route<Uri>> = vec![
            Route::new(
                b"".to_vec(),
                URI.parse::<Uri>().unwrap(),
            ),
        ];
    }

    #[test]
    fn test_prepare() {
        with_server(StatusCode::OK, FULFILL_BYTES.clone(), {
            let relay = Relay::new(ADDRESS.to_vec(), ROUTES.clone());
            let receiver = Receiver::new(relay);
            receiver
                .call(
                    hyper::Request::post(URI)
                        //.body(hyper::Body::from(&PREPARE_BYTES[..]))
                        .body(hyper::Body::from(&PREPARE_BYTES[..]))
                        .unwrap(),
                )
                .and_then(|res| {
                    assert_eq!(res.status(), 200);
                    assert_ne!(res.headers().get(header::CONTENT_LENGTH), None);
                    assert_eq!(
                        res.headers().get(header::CONTENT_TYPE).unwrap(),
                        "application/octet-stream",
                    );
                    res.into_body().concat2()
                })
                .map(|chunk| {
                    let body = Bytes::from(chunk).try_mut().unwrap();
                    assert_eq!(
                        ilp::Fulfill::try_from(BytesMut::from(body)).unwrap(),
                        FULFILL.build(),
                    );
                })
                //.wait()
                //.unwrap();
        });
    }

    #[test]
    fn test_bad_prepare() {
        let relay = Relay::new(ADDRESS.to_vec(), ROUTES.clone());
        let receiver = Receiver::new(relay);
        receiver
            .call(
                hyper::Request::post(URI)
                    .body(hyper::Body::from("not a packet".to_owned()))
                    .unwrap(),
            )
            .map(|res| {
                assert_eq!(res.status(), 400);
            })
            .wait()
            .unwrap();
    }

    // TODO body should be &[u8]
    fn with_server<F>(status_code: StatusCode, body: Vec<u8>, run_test: F)
    where
        F: 'static + Future<Item = ()> + Send,
    {
        //let relay = Arc::new(Relay::new());
        let server = hyper::Server::bind(&ADDR.into())
            .serve(move || {
                let body = body.clone();
                //let relay = Arc::clone(&relay);
                hyper::service::service_fn(move |_req| {
                    // TODO test request params?
                    //service::Service::call(&mut &*relay, req)
                    hyper::Response::builder()
                        .status(status_code)
                        .body(hyper::Body::from(body.clone()))
                        .into_future()
                })
            });

        //hyper::rt::run(
        let mut rt = Runtime::new().unwrap();
        rt.block_on(
            server
                .with_graceful_shutdown(run_test)
                .map_err(|err| {
                    panic!("server error: {}", err)
                })
        ).unwrap();
        rt.shutdown_now()
            .wait()
            .unwrap();

                //.wait();
        //);
    }
}
