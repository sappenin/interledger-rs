#![forbid(unsafe_code)]

mod relay;
mod receiver;
mod routes;
#[cfg(test)]
mod testing;

use futures::prelude::*;

pub use self::receiver::Receiver;
pub use self::relay::Relay;
pub use self::routes::Route;

pub trait Service {
    type Future: 'static + Send + Future<
        Item = ilp::Fulfill,
        Error = ilp::Reject,
    >;

    fn call(self, prepare: ilp::Prepare) -> Self::Future;
}

#[cfg(test)]
mod tests {
    use bytes::{Bytes, BytesMut};
    use futures::prelude::*;
    use hyper::{Uri, StatusCode};
    use hyper::header;
    use lazy_static::lazy_static;
    use tokio::runtime::Runtime;

    use crate::routes::Route;
    use crate::testing::{PREPARE, FULFILL};
    use super::*;

    static URI: &'static str = "http://127.0.0.1:3001/ilp";
    static ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 3001);

    lazy_static! {
        static ref PREPARE_BYTES: Vec<u8> = PREPARE.as_bytes().to_vec();
        static ref FULFILL_BYTES: Vec<u8> = FULFILL.as_bytes().to_vec();

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
                        *FULFILL,
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
