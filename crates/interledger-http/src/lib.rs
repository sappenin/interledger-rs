//! # interledger-http
//!
//! Client and server implementations of the [ILP-Over-HTTP](https://github.com/interledger/rfcs/blob/master/0035-ilp-over-http/0035-ilp-over-http.md) bilateral communication protocol.
//! This protocol is intended primarily for server-to-server communication between peers on the Interledger network.

#[macro_use]
extern crate log;

use futures::Future;
use interledger_service::Account;
use url::Url;

mod client;
mod server;

pub use self::client::HttpClientService;
pub use self::server::HttpServerService;

pub trait HttpAccount: Account {
    fn get_http_url(&self) -> Option<&Url>;
    fn get_http_auth_header(&self) -> Option<&str>;
}

/// The interface for Stores that can be used with the HttpServerService.
// TODO do we need all of these constraints?
pub trait HttpStore: Clone + Send + Sync + 'static {
    type Account: HttpAccount;

    /// Load account details based on the full HTTP Authorization header
    /// received on the incoming HTTP request.
    fn get_account_from_http_auth(
        &self,
        auth_header: &str,
    ) -> Box<Future<Item = Self::Account, Error = ()> + Send>;
}
