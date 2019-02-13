use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures::future::{Either, err};
use futures::prelude::*;
use hyper::{Request, Response, StatusCode};

use super::routes;
use super::Service;

type RoutingTable = routes::RoutingTable<hyper::Uri>;
type Route = routes::Route<hyper::Uri>;
type HyperClient = hyper::Client<hyper::client::HttpConnector, hyper::Body>;

// TODO separate Relay from Router? (router → service)
// TODO builder?
#[derive(Clone, Debug)]
pub struct Relay {
    data: Arc<RelayData>,
}

#[derive(Debug)]
struct RelayData {
    address: Vec<u8>,
    routes: RoutingTable,
    client: HyperClient,
}

impl Service for Relay {
    type Future = Box<
        dyn Future<
            Item = ilp::Fulfill,
            Error = ilp::Reject,
        > + Send + 'static,
    >;

    fn call(self, prepare: ilp::Prepare) -> Self::Future {
        Box::new(self.send_prepare(prepare))
    }
}

impl Relay {
    pub fn new(address: Vec<u8>, routes: Vec<Route>) -> Self {
        Relay {
            data: Arc::new(RelayData {
                address,
                routes: RoutingTable::new(routes),
                client: HyperClient::new(),
            }),
        }
    }

    //pub fn with_client(client: hyper::Client) -> Self {
    //}

    fn send_prepare(self, prepare: ilp::Prepare)
        -> impl Future<Item = ilp::Fulfill, Error = ilp::Reject>
    {
        match self.data.routes.resolve(prepare.destination()) {
            // TODO timeout (maybe hyper::client has builtin?)
            Some(route) => Either::A({
                let req = build_prepare_request(
                    route.next_hop(),
                    BytesMut::from(prepare).freeze(),
                );
                self.data.client
                    .request(req)
                    .then(move |res| {
                        match res {
                            Ok(res) => Either::A(self.decode_http_response(res)),
                            Err(_error) => Either::B(err(self.make_reject(
                                ilp::ErrorCode::T01_PEER_UNREACHABLE,
                                b"peer connection error",
                            ))),
                        }
                    })
            }),
            None => Either::B(err(self.make_reject(
                ilp::ErrorCode::F02_UNREACHABLE,
                b"no route found",
            ))),
        }
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
                            ilp::ErrorCode::F00_BAD_REQUEST,
                            b"unexpected response from peer",
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
                ilp::ErrorCode::F00_BAD_REQUEST,
                b"unexpected response from peer",
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
                ilp::ErrorCode::F00_BAD_REQUEST,
                b"invalid response from peer",
            )),
        }
    }

    fn make_reject(&self, code: ilp::ErrorCode, message: &[u8]) -> ilp::Reject {
        ilp::RejectBuilder {
            code,
            message,
            triggered_by: &self.data.address,
            data: b"",
        }.build()
    }
}

fn build_prepare_request(
    endpoint: &hyper::Uri,
    prepare: Bytes,
) -> Request<hyper::Body> {
    const CONTENT_TYPE: &'static [u8] = b"application/octet-stream";
    Request::builder()
        .method(hyper::Method::POST)
        .uri(endpoint)
        .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
        .body(hyper::Body::from(prepare))
        .expect("build_prepare_request builder error")
}
