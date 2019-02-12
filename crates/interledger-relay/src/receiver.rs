use std::marker::PhantomData;

use bytes::{Bytes, BytesMut};
use futures::future::{Either, FutureResult, err, ok};
use futures::prelude::*;
use hyper::{Chunk, StatusCode};

use super::{Request, Service};

#[derive(Clone, Debug)]
pub struct Receiver<S, Req> {
    next: S,
    _request: PhantomData<Req>,
}

impl<S, Req> Receiver<S, Req>
where
    S: Service<Req>,
    Req: Request,
{
    #[inline]
    fn new(next: S) -> Self {
        Receiver {
            next,
            _request: PhantomData,
        }
    }
}

const CONTENT_TYPE: &'static [u8] = b"application/octet-stream";

impl<S, Req> Receiver<S, Req>
where
    S: Service<Req>,
    Req: Request,
{
    // TODO impl Service?
    pub fn call(&self, req: hyper::Request<hyper::Body>)
        -> impl Future<
            Item = hyper::Response<hyper::Body>,
            Error = hyper::Error,
        > + 'static
    {
        let relay = self.clone();
        let body = req.into_body().concat2();
        body.and_then(move |chunk| {
            // `BytesMut::from(chunk)` calls `try_mut`, and only copies the data
            // if that fails (e.g. if the buffer is `KIND_STATIC`).
            match ilp::Prepare::try_from(BytesMut::from(chunk)) {
                Ok(prepare) => {
                    Either::A(
                    //relay.send_prepare(prepare.destination(), BytesMut::from(prepare).freeze())
                    relay.send_prepare(prepare)
                        .map(|res_packet| { // TODO maybe map?
                            let res_bytes = res_packet.into_bytes();
                            hyper::Response::builder()
                                .status(StatusCode::OK)
                                .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
                                .header(hyper::header::CONTENT_LENGTH, res_bytes.len())
                                .body(hyper::Body::from(res_bytes.freeze()))
                                .expect("response builder error")
                        })
                        .or_else(|error| {
                            println!("ERROR {:?}", error);
                            ok(hyper::Response::builder()
                                .status(StatusCode::BAD_GATEWAY)
                                .body(hyper::Body::from(format!("Error forwarding ILP Prepare: {}", error)))
                                .expect("response builder error"))
                        })
                    )
                },
                Err(error) => {
                    Either::B(
                    ok(hyper::Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(hyper::Body::from(format!("Error parsing ILP Prepare: {}", error)))
                        .expect("response builder error"))
                    )
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use futures::prelude::*;
    use hyper::Uri;
    use hyper::header;
    use lazy_static::lazy_static;
    use tokio::runtime::Runtime;

    use super::*;

    static URI: &'static str = "http://127.0.0.1:3001/ilp";

    //static EXECUTION_CONDITION = Bytes::
        //hex::decode("117b434f1a54e9044f4f54923b2cff9e4a6d420ae281d5025d7bb040c4b4c04a");

    //static URI: &'static str = "http://127.0.0.1:3001/ilp";
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
            relay
                .call(
                    hyper::Request::post(URI)
                        //.body(hyper::Body::from(&PREPARE_BYTES[..]))
                        .body(hyper::Body::from(PREPARE_BYTES.clone()))
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
        relay
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
