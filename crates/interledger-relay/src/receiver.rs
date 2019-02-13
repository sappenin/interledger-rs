use bytes::{Bytes, BytesMut};
use futures::future::{Either, ok};
use futures::prelude::*;
use hyper::StatusCode;

use super::Service;

// TODO impl Service?
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
    S: 'static + Clone + Service,
{
    pub fn call(&self, req: hyper::Request<hyper::Body>)
        -> impl Future<
            Item = hyper::Response<hyper::Body>,
            Error = hyper::Error,
        > + 'static
    {
        let next = self.next.clone();
        req
            .into_body()
            .concat2()
            .and_then(move |chunk| {
                let buffer = Bytes::from(chunk);
                // `BytesMut::from(chunk)` calls `try_mut`, and only copies the
                // data if that fails (e.g. if the buffer is `KIND_STATIC`).
                let buffer = BytesMut::from(buffer);
                match ilp::Prepare::try_from(buffer) {
                    Ok(prepare) => Either::A({
                        next.call(prepare)
                            .then(|res_packet| {
                                Ok(make_http_response(res_packet))
                            })
                    }),
                    Err(_error) => Either::B({
                        ok(hyper::Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(hyper::Body::from("Error parsing ILP Prepare"))
                            .expect("response builder error"))
                    }),
                }
            })
    }
}

fn make_http_response(packet: Result<ilp::Fulfill, ilp::Reject>)
    -> hyper::Response<hyper::Body>
{
    let buffer = match packet {
        Ok(fulfill) => BytesMut::from(fulfill),
        Err(reject) => BytesMut::from(reject),
    };
    hyper::Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
        .header(hyper::header::CONTENT_LENGTH, buffer.len())
        .body(hyper::Body::from(buffer.freeze()))
        .expect("response builder error")
}

#[cfg(test)]
mod test_receiver {
    use crate::testing::{IlpResult, MockService, PREPARE, FULFILL, REJECT};
    use super::*;

    static URI: &'static str = "http://example.com/ilp";

    #[test]
    fn test_call() {
        test_request(
            hyper::Request::post(URI)
                .body(hyper::Body::from(PREPARE.as_bytes()))
                .unwrap(),
            Ok(FULFILL.clone()),
        );
        test_request(
            hyper::Request::post(URI)
                .body(hyper::Body::from(PREPARE.as_bytes()))
                .unwrap(),
            Err(REJECT.clone()),
        );
    }

    fn test_request(
        request: hyper::Request<hyper::Body>,
        expect: IlpResult,
    ) {
        let next = MockService::new(expect.clone());
        let service = Receiver::new(next);

        let response = service.call(request).wait().unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "application/octet-stream",
        );

        let next = service.next.clone();
        assert_eq!(
            next.prepares(),
            vec![PREPARE.clone()],
        );

        let content_len = response.headers()
            .get("Content-Length").unwrap()
            .to_str().unwrap()
            .to_owned();
        let body = response
            .into_body()
            .concat2()
            .wait().unwrap();

        assert_eq!(content_len, body.len().to_string());
        assert_eq!(
            body.as_ref(),
            match &expect {
                Ok(ful) => ful.as_bytes(),
                Err(rej) => rej.as_bytes(),
            },
        );
    }
}
