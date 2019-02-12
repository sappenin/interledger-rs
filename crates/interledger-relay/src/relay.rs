// TODO remove unwraps or change to expect

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures::future::{Either, FutureResult, err, ok};
use futures::prelude::*;
use hyper::{Chunk, Request, Response, StatusCode};
use hyper::service::Service;

use super::routes;

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

/*
pub trait Service {
    type Future: Future<
        Item = IlpPrepare,
        Error = IlpReject,
    >;
    fn call(&self, prepare: &IlpPrepare) -> Self::Future;
}
*/

// TODO errorToReject?

// TODO IlpReject::from(hyper::Error)?

impl super::Service for Relay {
    type Future = Box<
        Future<
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
    //    RelayData {
    //        client,
    //    }
    //}

    fn send_prepare(self, prepare: ilp::Prepare)
        //-> impl Future<Item = IlpResponse, Error = hyper::Error>
        -> impl Future<Item = ilp::Fulfill, Error = ilp::Reject>
    {
        match self.data.routes.resolve(prepare.destination()) {
            // TODO timeout (maybe hyper::client has builtin?)
            Some(route) => {
                let req = build_prepare_request(
                    route.next_hop(),
                    BytesMut::from(prepare).freeze(),
                );
                Either::A(self.data.client
                    .request(req)
                    .then(move |res| {
                        match res {
                            // XXX extra either!
                            Ok(res) => Either::A(self.decode_http_response(res)),
                            Err(_error) => Either::B(err(self.make_reject(
                                ilp::ErrorCode::T01_PEER_UNREACHABLE,
                                b"peer error", // TODO what error here?
                            ))),
                        }
                    }))
            },
            None => {
                Either::B(err(ilp::RejectBuilder {
                    code: ilp::ErrorCode::F02_UNREACHABLE,
                    message: b"no route found",
                    triggered_by: &self.data.address,
                    data: b"",
                }.build()))
            },
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
            // TODO or as errors?
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
    prepare: Bytes, // TODO maybe this should be generic with Body::from
) -> Request<hyper::Body> {
    const CONTENT_TYPE: &'static [u8] = b"application/octet-stream";
    Request::builder()
        .method(hyper::Method::POST)
        .uri(endpoint)
        .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
        //"Content-Type", 
        .body(hyper::Body::from(prepare))
        .expect("build_prepare_request builder error")
}

/*impl<'a> Service for &'a Relay {
    type ReqBody = hyper::Body;
    type ResBody = hyper::Body;
    type Error = hyper::Error;
    //type Future = FutureResult<Response<Self::ResBody>, Self::Error>;
    /*type Future = futures::future::Map<
        futures::stream::Concat2<Self::ReqBody>,
        fn(chunk: Chunk) -> Response<Self::ResBody>,
    >;*/
    type Future = Box<dyn Future<
        Item = Response<Self::ResBody>,
        Error = Self::Error,
    > + Send + 'a>; // XXX lifetime here?*/

//impl Relay {
//    // TODO impl Service?
//    pub fn call(&self, req: Request<hyper::Body>)
//        -> impl Future<
//            Item = Response<hyper::Body>,
//            Error = hyper::Error,
//        > + 'static
//    {
//        let relay = self.clone();
//        let body = req.into_body().concat2();
//        body.and_then(move |chunk| {
//            let chunk = Bytes::from(chunk).try_mut().unwrap(); // XXX dont unwrap (KIND_STATIC)
//            match ilp::Prepare::try_from(chunk) {
//                Ok(prepare) => {
//                    Either::A(
//                    //relay.send_prepare(prepare.destination(), BytesMut::from(prepare).freeze())
//                    relay.send_prepare(prepare)
//                        .map(|res_packet| { // TODO maybe map?
//                            let res_bytes = res_packet.into_bytes();
//                            Response::builder()
//                                .status(StatusCode::OK)
//                                .header(hyper::header::CONTENT_TYPE, CONTENT_TYPE)
//                                .header(hyper::header::CONTENT_LENGTH, res_bytes.len())
//                                .body(hyper::Body::from(res_bytes.freeze()))
//                                .expect("response builder error")
//                        })
//                        .or_else(|error| {
//                            println!("ERROR {:?}", error);
//                            ok(Response::builder()
//                                .status(StatusCode::BAD_GATEWAY)
//                                .body(hyper::Body::from(format!("Error forwarding ILP Prepare: {}", error)))
//                                .expect("response builder error"))
//                        })
//                    )
//                },
//                Err(error) => {
//                    Either::B(
//                    ok(Response::builder()
//                        .status(StatusCode::BAD_REQUEST)
//                        .body(hyper::Body::from(format!("Error parsing ILP Prepare: {}", error)))
//                        .expect("response builder error"))
//                    )
//                },
//            }
//        })
//    }
//}

// TODO Receiver or maybe generic onto ilp service
// TODO does this have to be on &'a
/*
impl<'a> Service for &'a Relay {
    type ReqBody = hyper::Body;
    type ResBody = hyper::Body;
    type Error = hyper::Error;
    //type Future = FutureResult<Response<Self::ResBody>, Self::Error>;
    /*type Future = futures::future::Map<
        futures::stream::Concat2<Self::ReqBody>,
        fn(chunk: Chunk) -> Response<Self::ResBody>,
    >;*/
    type Future = Box<dyn Future<
        Item = Response<Self::ResBody>,
        Error = Self::Error,
    > + Send + 'a>; // XXX lifetime here?

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        let selff = self.clone(); // XXX
        let body = req.into_body().concat2();
        let res = body.and_then(move |chunk| {
            match IlpPrepare::from_bytes(&chunk) {
                Ok(prepare) => {
                    Either::A(
                    selff.send_prepare(prepare.destination.into_bytes(), chunk)
                        .and_then(|res_packet| { // TODO maybe map?
                            ok(Response::builder()
                                .status(StatusCode::OK)
                                .body(hyper::Body::from(res_packet.into_bytes()))
                                .expect("response builder error"))
                        })
                        .or_else(|error| {
                            ok(Response::builder()
                                .status(StatusCode::BAD_GATEWAY)
                                .body(hyper::Body::from(format!("Error forwarding ILP Prepare: {}", error)))
                                .expect("response builder error"))
                        })
                    )
                    //Response::builder()
                    //    .status(StatusCode::OK)
                    //    .body(hyper::Body::from(format!("Hello World amount={:?}!\n", prepare.amount)))
                },
                Err(error) => {
                    Either::B(
                    ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(hyper::Body::from(format!("Error parsing ILP Prepare: {}", error)))
                        .expect("response builder error"))
                    )
                },
            }
        });
        Box::new(res)
    }
}
*/
