use std::cmp;
use std::sync::Arc;
use std::time;

use bytes::{Bytes, BytesMut};
use futures::future::{Either, err};
use futures::prelude::*;
use http::request::Builder as RequestBuilder;
use hyper::{Response, StatusCode};
use tokio::util::FutureExt;

type HyperClient = hyper::Client<hyper::client::HttpConnector, hyper::Body>;

/// The maximum duration that the outgoing HTTP client will wait for a response,
/// even if the Prepare's expiry is longer.
const DEFAULT_MAX_TIMEOUT: time::Duration = time::Duration::from_secs(60);

static OCTET_STREAM: &'static [u8] = b"application/octet-stream";

#[derive(Clone, Debug)]
pub struct Client {
    address: Bytes,
    hyper: Arc<HyperClient>,
    max_timeout: time::Duration,
}

#[derive(Clone, Debug)]
pub struct ClientBuilder {
    address: Bytes,
    client: Option<HyperClient>,
    max_timeout: time::Duration,
}

impl Client {
    pub fn address(&self) -> &Bytes {
        &self.address
    }

    /// `req_builder` is the request template. At a minimum, it should have the
    /// URI and method set.
    pub fn request(self, mut req_builder: RequestBuilder, prepare: ilp::Prepare)
        -> impl Future<Item = ilp::Fulfill, Error = ilp::Reject>
    {
        let expires_at = prepare.expires_at();
        let expires_in = expires_at.duration_since(time::SystemTime::now());
        let expires_in = match expires_in {
            Ok(expires_in) => expires_in,
            Err(_) => return Either::B(err(self.make_reject(
                ilp::ErrorCode::R02_INSUFFICIENT_TIMEOUT,
                b"insufficient timeout",
            ))),
        };

        let prepare_bytes = BytesMut::from(prepare).freeze();
        let req = req_builder
            .header(hyper::header::CONTENT_TYPE, OCTET_STREAM)
            .body(hyper::Body::from(prepare_bytes))
            .expect("build_prepare_request builder error");

        Either::A(self.hyper
            .request(req)
            .timeout(cmp::min(self.max_timeout, expires_in))
            .then(move |res| {
                match res {
                    Ok(res) => Either::A(self.decode_http_response(res)),
                    Err(error) => Either::B(err({
                        if error.is_elapsed() {
                            self.make_reject(
                                ilp::ErrorCode::R00_TRANSFER_TIMED_OUT,
                                b"request timed out",
                            )
                        } else {
                            self.make_reject(
                                ilp::ErrorCode::T01_PEER_UNREACHABLE,
                                b"peer connection error",
                            )
                        }
                    })),
                }
            }))
    }

    fn decode_http_response(self, res: Response<hyper::Body>)
        -> impl Future<Item = ilp::Fulfill, Error = ilp::Reject>
    {
        let status = res.status();
        if status == StatusCode::OK {
            Either::A(res.into_body()
                .concat2()
                .then(move |body| {
                    match body {
                        Ok(body) => {
                            let body = BytesMut::from(Bytes::from(body));
                            self.decode_response(body).into_future()
                        },
                        Err(_error) => err(self.make_reject(
                            ilp::ErrorCode::T00_INTERNAL_ERROR,
                            b"invalid response body from peer",
                        )),
                    }
                }))
        } else if status.is_client_error() {
            Either::B(err(self.make_reject(
                ilp::ErrorCode::F00_BAD_REQUEST,
                b"bad request to peer",
            )))
        } else if status.is_server_error() {
            Either::B(err(self.make_reject(
                ilp::ErrorCode::T01_PEER_UNREACHABLE,
                b"peer internal error",
            )))
        } else {
            Either::B(err(self.make_reject(
                ilp::ErrorCode::T00_INTERNAL_ERROR,
                b"unexpected response code from peer",
            )))
        }
    }

    fn decode_response(&self, bytes: BytesMut)
        -> Result<ilp::Fulfill, ilp::Reject>
    {
        match ilp::Packet::try_from(bytes) {
            Ok(ilp::Packet::Fulfill(fulfill)) => Ok(fulfill),
            Ok(ilp::Packet::Reject(reject)) => Err(reject),
            _ => Err(self.make_reject(
                ilp::ErrorCode::T00_INTERNAL_ERROR,
                b"invalid response body from peer",
            )),
        }
    }

    fn make_reject(&self, code: ilp::ErrorCode, message: &[u8]) -> ilp::Reject {
        ilp::RejectBuilder {
            code,
            message,
            triggered_by: &self.address,
            data: b"",
        }.build()
    }
}

impl ClientBuilder {
    pub fn new(address: Vec<u8>) -> Self {
        ClientBuilder {
            address: Bytes::from(address),
            client: None,
            max_timeout: DEFAULT_MAX_TIMEOUT,
        }
    }

    pub fn build(self) -> Client {
        Client {
            address: self.address,
            hyper: Arc::new(self.client.unwrap_or_else(HyperClient::new)),
            max_timeout: self.max_timeout,
        }
    }

    pub fn with_client(mut self, client: HyperClient) -> Self {
        self.client = Some(client);
        self
    }

    pub fn with_max_timeout(mut self, max_timeout: time::Duration) -> Self {
        self.max_timeout = max_timeout;
        self
    }
}

#[cfg(test)]
mod tests {
    use lazy_static::lazy_static;

    use crate::testing::{self, RECEIVER_ORIGIN};
    use super::*;

    static ADDRESS: &'static [u8] = b"example.connector";
    const MAX_TIMEOUT: time::Duration = time::Duration::from_millis(15);

    lazy_static! {
        static ref CLIENT: Client = ClientBuilder::new(ADDRESS.to_vec())
            .build();

        static ref CLIENT_HTTP2: Client = ClientBuilder::new(ADDRESS.to_vec())
            .with_client(
                hyper::Client::builder()
                    .http2_only(true)
                    .build_http(),
            )
            .build();

        static ref CLIENT_WITH_TIMEOUT: Client =
            ClientBuilder::new(ADDRESS.to_vec())
                .with_max_timeout(MAX_TIMEOUT)
                .build();
    }

    fn make_request() -> RequestBuilder {
        let mut builder = hyper::Request::builder();
        builder.method(hyper::Method::POST);
        builder.uri(RECEIVER_ORIGIN);
        builder.header("Authorization", "bob_auth");
        builder
    }

    #[test]
    fn test_outgoing_request() {
        testing::MockServer::new()
            .test_request(|req| {
                assert_eq!(req.method(), hyper::Method::POST);
                assert_eq!(req.uri().path(), "/");
                assert_eq!(
                    req.headers().get("Authorization").unwrap(),
                    "bob_auth",
                );
                assert_eq!(
                    req.headers().get("Content-Type").unwrap(),
                    "application/octet-stream",
                );
            })
            .test_body(|body| {
                assert_eq!(body.as_ref(), testing::PREPARE.as_bytes());
            })
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                CLIENT.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        assert_eq!(result.unwrap(), *testing::FULFILL);
                        Ok(())
                    })
            });
    }

    #[test]
    fn test_outgoing_max_timeout() {
        testing::MockServer::new()
            .with_delay(MAX_TIMEOUT * 2)
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                CLIENT_WITH_TIMEOUT.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        let reject = result.unwrap_err();
                        assert_eq!(reject.code(), ilp::ErrorCode::R00_TRANSFER_TIMED_OUT);
                        assert_eq!(reject.message(), b"request timed out");
                        Ok(())
                    })
            })
    }

    #[test]
    fn test_outgoing_prepare_expiry() {
        // Create a `prepare` with a short expiry.
        let mut prepare = testing::PREPARE.clone();
        let soon = time::Duration::from_millis(100);
        prepare.set_expires_at(time::SystemTime::now() + soon);

        testing::MockServer::new()
            .with_delay(time::Duration::from_millis(101))
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                CLIENT.clone()
                    .request(make_request(), prepare)
                    .then(|result| -> Result<(), ()> {
                        let reject = result.unwrap_err();
                        assert_eq!(reject.code(), ilp::ErrorCode::R00_TRANSFER_TIMED_OUT);
                        assert_eq!(reject.message(), b"request timed out");
                        Ok(())
                    })
            })
    }

    #[test]
    fn test_outgoing_http2_only() {
        testing::MockServer::new()
            .test_request(|req| {
                assert_eq!(req.version(), hyper::Version::HTTP_2);
            })
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                CLIENT_HTTP2.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        assert_eq!(result.unwrap(), *testing::FULFILL);
                        Ok(())
                    })
            });
    }

    #[test]
    fn test_incoming_reject() {
        testing::MockServer::new()
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::REJECT.as_bytes()))
                    .unwrap()
            })
            .run({
                CLIENT.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        assert_eq!(result.unwrap_err(), *testing::REJECT);
                        Ok(())
                    })
            });
    }

    #[test]
    fn test_incoming_invalid_packet() {
        let expect_reject = ilp::RejectBuilder {
            code: ilp::ErrorCode::T00_INTERNAL_ERROR,
            message: b"invalid response body from peer",
            triggered_by: ADDRESS,
            data: b"",
        }.build();
        testing::MockServer::new()
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(&b"this is not a packet"[..]))
                    .unwrap()
            })
            .run({
                CLIENT.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(move |result| -> Result<(), ()> {
                        assert_eq!(result.unwrap_err(), expect_reject);
                        Ok(())
                    })
            });
    }

    macro_rules! make_test_incoming_error_code {
        (
            fn $fn:ident(
                status_code: $status_code:expr,
                error_code: $error_code:expr,
                error_message: $error_message:expr $(,)?
            );
        ) => {
            #[test]
            fn $fn() {
                let expect_reject = ilp::RejectBuilder {
                    code: $error_code,
                    message: $error_message,
                    triggered_by: ADDRESS,
                    data: b"",
                }.build();
                testing::MockServer::new()
                    .with_response(|| {
                        hyper::Response::builder()
                            .status($status_code)
                            .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                            .unwrap()
                    })
                    .run({
                        CLIENT.clone()
                            .request(make_request(), testing::PREPARE.clone())
                            .then(move |result| -> Result<(), ()> {
                                assert_eq!(result.unwrap_err(), expect_reject);
                                Ok(())
                            })
                    });
            }
        };
    }

    make_test_incoming_error_code! {
        fn test_incoming_300(
            status_code: 300,
            error_code: ilp::ErrorCode::T00_INTERNAL_ERROR,
            error_message: b"unexpected response code from peer",
        );
    }

    make_test_incoming_error_code! {
        fn test_incoming_400(
            status_code: 400,
            error_code: ilp::ErrorCode::F00_BAD_REQUEST,
            error_message: b"bad request to peer",
        );
    }

    make_test_incoming_error_code! {
        fn test_incoming_500(
            status_code: 500,
            error_code: ilp::ErrorCode::T01_PEER_UNREACHABLE,
            error_message: b"peer internal error",
        );
    }

    #[test]
    fn test_incoming_abort() {
        let expect_reject = ilp::RejectBuilder {
            code: ilp::ErrorCode::T01_PEER_UNREACHABLE,
            message: b"peer connection error",
            triggered_by: ADDRESS,
            data: b"",
        }.build();
        testing::MockServer::new()
            .with_abort()
            .run({
                CLIENT.clone()
                    .request(make_request(), testing::PREPARE.clone())
                    .then(move |result| -> Result<(), ()> {
                        assert_eq!(result.unwrap_err(), expect_reject);
                        Ok(())
                    })
            });
    }
}
