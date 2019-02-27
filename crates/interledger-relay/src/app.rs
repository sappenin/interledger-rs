//use std::net;

use crate::{ClientBuilder, Route};
use crate::middlewares::{AuthToken, AuthTokenFilter, MethodFilter, Receiver};
use crate::services::relay::Relay;

#[derive(Clone, Debug)]
pub struct ConnectorBuilder {
    pub ilp_addr: Vec<u8>,
    pub auth_tokens: Vec<AuthToken>,
    pub routes: Vec<Route>,
}

type Connector = MethodFilter<
    AuthTokenFilter<
        Receiver<Relay>,
    >,
>;

impl ConnectorBuilder {
    pub fn build(self) -> Connector {
        let client = ClientBuilder::new(self.ilp_addr).build();
        let relay = Relay::new(client, self.routes);
        let receiver = Receiver::new(relay);
        let auth_filter = AuthTokenFilter::new(self.auth_tokens, receiver);
        MethodFilter::new(hyper::Method::POST, auth_filter)
    }
}

/* TODO
pub struct Config {
    pub net_addr: net::SocketAddr,
    pub ilp_addr: Vec<u8>,
    pub routes: Vec<Route<hyper::Uri>>,
}
*/

#[cfg(test)]
mod test_connector_builder {
    use futures::prelude::*;

    use crate::testing::{self, FULFILL, PREPARE};
    use super::*;

    static CONNECTOR_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 3002);

    #[test]
    fn test_relay() {
        let connector = ConnectorBuilder {
            ilp_addr: b"example.alice".to_vec(),
            auth_tokens: vec![AuthToken::new(b"secret".to_vec())],
            routes: testing::ROUTES.clone(),
        }.build();

        let request = hyper::Client::new()
            .request({
                hyper::Request::post("http://127.0.0.1:3002/ilp")
                    .header("Authorization", "secret")
                    .body(hyper::Body::from(PREPARE.as_bytes()))
                    .unwrap()
            })
            .and_then(|response| {
                assert_eq!(response.status(), 200);
                response.into_body().concat2()
            })
            .map(|body| {
                assert_eq!(body.as_ref(), FULFILL.as_bytes());
            });

        testing::MockServer::new()
            .test_request(|req| {
                assert_eq!(req.method(), hyper::Method::POST);
                assert_eq!(req.uri().path(), "/bob");
                assert_eq!(
                    req.headers().get("Content-Type").unwrap(),
                    "application/octet-stream",
                );
            })
            .test_body(|body| {
                assert_eq!(body.as_ref(), PREPARE.as_bytes());
            })
            .with_response(|| {
                hyper::Response::builder()
                    .status(200)
                    .body(hyper::Body::from(FULFILL.as_bytes()))
                    .unwrap()
            })
            .run({
                hyper::Server::bind(&CONNECTOR_ADDR.into())
                    .serve(move || -> Result<_, &'static str> {
                        Ok(connector.clone())
                    })
                    .with_graceful_shutdown(request)
            });
    }
}
