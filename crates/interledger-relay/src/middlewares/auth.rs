use std::borrow::Borrow;
use std::collections::HashSet;

use futures::future::{Either, FutureResult, ok};
use hyper::service::Service as HyperService;

/// Verify that incoming requests have a valid token in the `Authorization` header.
#[derive(Clone, Debug)]
pub struct AuthTokenFilter<S> {
    tokens: HashSet<AuthToken>,
    next: S,
}

impl<S> AuthTokenFilter<S>
where
    S: HyperService<
        ReqBody = hyper::Body,
        ResBody = hyper::Body,
        Error = hyper::Error,
    >,
{
    pub fn new(tokens: Vec<AuthToken>, next: S) -> Self {
        AuthTokenFilter {
            tokens: tokens.into_iter().collect::<HashSet<_>>(),
            next,
        }
    }
}

impl<S> HyperService for AuthTokenFilter<S>
where
    S: HyperService<
        ReqBody = hyper::Body,
        ResBody = hyper::Body,
        Error = hyper::Error,
    >,
{
    type ReqBody = hyper::Body;
    type ResBody = hyper::Body;
    type Error = hyper::Error;
    type Future = Either<
        S::Future,
        // TODO the FutureResult's Error could be Never.
        FutureResult<hyper::Response<hyper::Body>, hyper::Error>,
    >;

    fn call(&mut self, request: hyper::Request<hyper::Body>) -> Self::Future {
        let auth = request.headers().get(hyper::header::AUTHORIZATION);
        match auth {
            Some(token) if self.tokens.contains(token.as_ref()) => {
                Either::A(self.next.call(request))
            },
            _ => Either::B(ok(
                hyper::Response::builder()
                    .status(hyper::StatusCode::UNAUTHORIZED)
                    .body(hyper::Body::empty())
                    .expect("response builder error")
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AuthToken(Vec<u8>);

impl AuthToken {
    pub fn new(token: Vec<u8>) -> Self {
        AuthToken(token)
    }
}

impl Borrow<[u8]> for AuthToken {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use futures::prelude::*;
    use hyper::service::service_fn;

    use super::*;

    #[test]
    fn test_service() {
        let next = service_fn(|_req| {
            ok(hyper::Response::builder()
                .status(200)
                .body(hyper::Body::empty())
                .unwrap())
        });
        let mut service = AuthTokenFilter::new(
            vec![
                AuthToken::new(b"token_1".to_vec()),
                AuthToken::new(b"token_2".to_vec()),
            ],
            next,
        );

        // Correct token.
        assert_eq!(
            service.call({
                hyper::Request::post("/")
                    .header("Authorization", "token_1")
                    .body(hyper::Body::empty())
                    .unwrap()
            }).wait().unwrap().status(),
            200,
        );

        // No token.
        assert_eq!(
            service.call({
                hyper::Request::post("/")
                    .body(hyper::Body::empty())
                    .unwrap()
            }).wait().unwrap().status(),
            401,
        );

        // Incorrect token.
        assert_eq!(
            service.call({
                hyper::Request::post("/")
                    .header("Authorization", "not_a_token")
                    .body(hyper::Body::empty())
                    .unwrap()
            }).wait().unwrap().status(),
            401,
        );
    }
}
