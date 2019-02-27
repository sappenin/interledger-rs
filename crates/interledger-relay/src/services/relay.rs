use std::sync::{Arc, RwLock};

use bytes::Bytes;
use futures::future::{Either, err};
use futures::prelude::*;
use hyper::{Request, Uri};

use crate::Service;
use crate::client::Client;
use crate::routes;

type RoutingTable = routes::RoutingTable<NextHop>;
pub type Route = routes::Route<NextHop>;

// TODO rename Relay to: RouterRelay? RelayRouter
#[derive(Clone, Debug)]
pub struct Relay {
    data: Arc<RelayData>,
    client: Client,
}

#[derive(Debug)]
struct RelayData {
    address: Bytes,
    routes: RwLock<RoutingTable>,
}

impl Service for Relay {
    type Future = Box<
        dyn Future<
            Item = ilp::Fulfill,
            Error = ilp::Reject,
        > + Send + 'static,
    >;

    fn call(self, prepare: ilp::Prepare) -> Self::Future {
        Box::new(self.forward(prepare))
    }
}

impl Relay {
    pub fn new(client: Client, routes: Vec<Route>) -> Self {
        Relay {
            data: Arc::new(RelayData {
                address: client.address().clone(),
                routes: RwLock::new(RoutingTable::new(routes)),
            }),
            client,
        }
    }

    /// Replace the routing table.
    pub fn set_routes(&self, new_routes: Vec<Route>) {
        let mut routes = self.data.routes.write().unwrap();
        *routes = RoutingTable::new(new_routes);
    }

    fn forward(self, prepare: ilp::Prepare)
        -> impl Future<Item = ilp::Fulfill, Error = ilp::Reject>
    {
        let routes = self.data.routes.read().unwrap();
        let route = match routes.resolve(prepare.destination()) {
            Some(route) => route,
            None => return Either::B(err(self.make_reject(
                ilp::ErrorCode::F02_UNREACHABLE,
                b"no route found",
            ))),
        };

        let next_hop = route.next_hop();
        let mut builder = Request::builder();
        builder.method(hyper::Method::POST);
        builder.uri(&next_hop.endpoint);
        if let Some(auth) = &next_hop.auth {
            builder.header(hyper::header::AUTHORIZATION, auth.clone());
        }

        std::mem::drop(routes);
        Either::A(self.client.request(builder, prepare))
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

#[derive(Clone, Debug)]
pub struct NextHop {
    endpoint: Uri,
    auth: Option<Bytes>,
}

impl NextHop {
    pub fn new(endpoint: Uri, auth: Option<Vec<u8>>) -> Self {
        NextHop {
            endpoint,
            auth: auth.map(Bytes::from),
        }
    }
}

#[cfg(test)]
mod test_relay {
    use lazy_static::lazy_static;

    use crate::client::ClientBuilder;
    use crate::testing::{self, RECEIVER_ORIGIN, ROUTES};
    use super::*;

    static ADDRESS: &'static [u8] = b"example.relay";

    lazy_static! {
        static ref CLIENT: Client = ClientBuilder::new(ADDRESS.to_vec()).build();
        static ref RELAY: Relay = Relay::new(CLIENT.clone(), ROUTES.clone());
    }

    #[test]
    fn test_outgoing_request() {
        testing::MockServer::new()
            .test_request(|req| {
                assert_eq!(req.method(), hyper::Method::POST);
                assert_eq!(req.uri().path(), "/bob");
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
                RELAY.clone()
                    .call(testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        assert_eq!(result.unwrap(), *testing::FULFILL);
                        Ok(())
                    })
            });
    }

    #[test]
    fn test_no_route() {
        let expect_reject = ilp::RejectBuilder {
            code: ilp::ErrorCode::F02_UNREACHABLE,
            message: b"no route found",
            triggered_by: b"example.relay",
            data: b"",
        }.build();
        let relay = Relay::new(CLIENT.clone(), vec![ROUTES[0].clone()]);
        testing::MockServer::new().run({
            relay
                .call(testing::PREPARE.clone())
                .then(move |result| -> Result<(), ()> {
                    assert_eq!(result.unwrap_err(), expect_reject);
                    Ok(())
                })
        });
    }

    #[test]
    fn test_set_routes() {
        let relay = RELAY.clone();
        relay.set_routes(vec![
            Route::new(
                b"test.bob.".to_vec(),
                NextHop::new(
                    format!("{}/new_bob", RECEIVER_ORIGIN).parse::<Uri>().unwrap(),
                    None,
                ),
            ),
        ]);
        testing::MockServer::new()
            .test_request(|req| {
                assert_eq!(req.uri().path(), "/new_bob");
            })
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(testing::FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                relay
                    .call(testing::PREPARE.clone())
                    .then(|result| -> Result<(), ()> {
                        assert_eq!(result.unwrap(), *testing::FULFILL);
                        Ok(())
                    })
            });
    }
}
