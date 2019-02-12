use bytes::{Bytes, BytesMut};
use futures::future::{Either, FutureResult, err, ok};
use futures::prelude::*;
use hyper::{Chunk, StatusCode};

use super::Service;

#[derive(Clone, Debug)]
pub struct Receiver<S> {
    next: S,
}

impl<S> Receiver<S>
where
    S: Service,
{
    #[inline]
    pub fn new(next: S) -> Self {
        Receiver { next }
    }
}

const CONTENT_TYPE: &'static [u8] = b"application/octet-stream";

impl<S> Receiver<S>
where
    S: 'static + Service,
{
    // TODO impl Service?
    pub fn call(&self, req: hyper::Request<hyper::Body>)
        -> impl Future<
            Item = hyper::Response<hyper::Body>,
            Error = hyper::Error,
        > + 'static
    {
        let next = self.next.clone();
        let body = req.into_body().concat2();
        body.and_then(move |chunk| {
            // `BytesMut::from(chunk)` calls `try_mut`, and only copies the data
            // if that fails (e.g. if the buffer is `KIND_STATIC`).
            match ilp::Prepare::try_from(BytesMut::from(Bytes::from(chunk))) {
                Ok(prepare) => {
                    Either::A(
                    //relay.send_prepare(prepare.destination(), BytesMut::from(prepare).freeze())
                    next.call(prepare)
                        .then(|res_packet| { // TODO maybe map?
                            let res_buffer = match res_packet {
                                Ok(fulfill) => BytesMut::from(fulfill),
                                Err(reject) => BytesMut::from(reject),
                            };
                            //let fulfill = BytesMut::from(res_packet.into_bytes();
                            Ok(hyper::Response::builder()
                                .status(StatusCode::OK)
                                .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
                                .header(hyper::header::CONTENT_LENGTH, res_buffer.len())
                                .body(hyper::Body::from(res_buffer.freeze()))
                                .expect("response builder error"))
                        })
                        //.or_else(|reject| {
                        //    //println!("ERROR {:?}", error);
                        //    //ok(hyper::Response::builder()
                        //    //    .status(StatusCode::BAD_GATEWAY)
                        //    //    .body(hyper::Body::from(format!("Error forwarding ILP Prepare: {}", error)))
                        //    //    .expect("response builder error"))
                        //})
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
