#![recursion_limit = "256"]
#[macro_use]
extern crate tower_web;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_json;

use bytes::Bytes;
use futures::{
    future::{err, ok, result, Either},
    Future,
};
use http::{Request, Response};
use hyper::{body::Body, error::Error};
use interledger_http::{HttpAccount, HttpServerService, HttpStore};
use interledger_ildcp::IldcpAccount;
use interledger_router::RouterStore;
use interledger_service::{Account as AccountTrait, IncomingService};
use interledger_service_util::BalanceStore;
use interledger_spsp::{pay, SpspResponder};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    iter::FromIterator,
    str::{self, FromStr},
};

pub trait NodeAccount: HttpAccount {
    fn is_admin(&self) -> bool;
}

pub trait NodeStore: Clone + Send + Sync + 'static {
    type Account: AccountTrait;

    fn insert_account(
        &self,
        account: AccountDetails,
    ) -> Box<Future<Item = Self::Account, Error = ()> + Send>;

    // TODO limit the number of results and page through them
    fn get_all_accounts(&self) -> Box<Future<Item = Vec<Self::Account>, Error = ()> + Send>;

    fn set_rates<R>(&self, rates: R) -> Box<Future<Item = (), Error = ()> + Send>
    where
        R: IntoIterator<Item = (String, f64)>;

    fn set_static_routes<R>(&self, routes: R) -> Box<Future<Item = (), Error = ()> + Send>
    where
        R: IntoIterator<Item = (String, <Self::Account as AccountTrait>::AccountId)>;

    fn set_static_route(
        &self,
        prefix: String,
        account_id: <Self::Account as AccountTrait>::AccountId,
    ) -> Box<Future<Item = (), Error = ()> + Send>;
}

/// The Account type for the RedisStore.
#[derive(Debug, Extract, Response, Clone)]
pub struct AccountDetails {
    pub ilp_address: Vec<u8>,
    pub asset_code: String,
    pub asset_scale: u8,
    pub max_packet_amount: u64,
    #[serde(default = "i64::min_value")]
    pub min_balance: i64,
    pub http_endpoint: Option<String>,
    pub http_incoming_authorization: Option<String>,
    pub http_outgoing_authorization: Option<String>,
    pub btp_uri: Option<String>,
    pub btp_incoming_authorization: Option<String>,
    pub is_admin: bool,
    pub xrp_address: Option<String>,
    pub settle_threshold: Option<i64>,
    pub settle_to: Option<i64>,
    #[serde(default)]
    pub send_routes: bool,
    #[serde(default)]
    pub receive_routes: bool,
    pub routing_relation: Option<String>,
}

#[derive(Response)]
#[web(status = "200")]
struct ServerStatus {
    status: String,
}

#[derive(Serialize, Response)]
#[web(status = "200")]
struct AccountsResponse<A: Serialize> {
    accounts: Vec<A>,
}

#[derive(Response)]
#[web(status = "200")]
struct Success;

#[derive(Extract)]
struct Rates(Vec<(String, f64)>);

#[derive(Response)]
#[web(status = "200")]
struct BalanceResponse {
    balance: String,
}

#[derive(Extract)]
struct SpspPayRequest {
    receiver: String,
    source_amount: u64,
}

#[derive(Response)]
#[web(status = "200")]
struct SpspPayResponse {
    amount_delivered: u64,
}

#[derive(Response)]
#[web(status = "200")]
struct SpspQueryResponse {
    destination_account: String,
    shared_secret: String,
}

#[derive(Extract, Response)]
#[web(status = "200")]
struct Routes(HashMap<String, String>);

pub struct NodeApi<T, S> {
    store: T,
    incoming_handler: S,
    server_secret: Bytes,
}

impl_web! {
    impl<T, S, A> NodeApi<T, S>
    where T: NodeStore<Account = A> + HttpStore<Account = A> + BalanceStore<Account = A> + RouterStore,
    S: IncomingService<A> + Clone + Send + Sync + 'static,
    A: AccountTrait + HttpAccount + NodeAccount + IldcpAccount + Serialize + 'static,

    {
        pub fn new(server_secret: Bytes, store: T, incoming_handler: S) -> Self {
            NodeApi {
                store,
                incoming_handler,
                server_secret,
            }
        }

        fn validate_admin(&self, authorization: String) -> impl Future<Item = T, Error = Response<()>> {
            let store = self.store.clone();
            self.store.get_account_from_http_auth(&authorization)
                .and_then(|account| if account.is_admin() {
                    Ok(store)
                } else {
                    Err(())
                })
                .map_err(|_| Response::builder().status(401).body(()).unwrap())
        }

        #[get("/")]
        #[content_type("application/json")]
        fn get_root(&self) -> Result<ServerStatus, ()> {
            Ok(ServerStatus {
                status: "Ready".to_string(),
            })
        }

        #[post("/accounts")]
        #[content_type("application/json")]
        fn post_accounts(&self, body: AccountDetails, authorization: String) -> impl Future<Item = Value, Error = Response<()>> {
            // TODO don't allow accounts to be overwritten
            // TODO add option for non-admin signups (maybe with invite code)
            self.validate_admin(authorization)
                .and_then(move |store| store.insert_account(body)
                // TODO make all Accounts (de)serializable with Serde so all the details can be returned here
                .and_then(|account| Ok(json!(account)))
                .map_err(|_| Response::builder().status(500).body(()).unwrap()))
        }

        #[get("/accounts")]
        #[content_type("application/json")]
        fn get_accounts(&self, authorization: String) -> impl Future<Item = Value, Error = Response<()>> {
            let store = self.store.clone();
            self.store.get_account_from_http_auth(&authorization)
                .map_err(move |_| {
                    debug!("No account found with auth: {}", authorization);
                    Response::builder().status(401).body(()).unwrap()
                })
                .and_then(move |account| if account.is_admin() {
                    Either::A(store.get_all_accounts()
                        .map_err(|_| Response::builder().status(500).body(()).unwrap()))
                } else {
                    Either::B(store.get_accounts(vec![account.id()])
                        .map_err(|_| Response::builder().status(404).body(()).unwrap()))
                })
                .and_then(|accounts| Ok(json!(accounts)))
        }

        #[get("/accounts/:id")]
        #[content_type("application/json")]
        fn get_account(&self, id: String, authorization: String) -> impl Future<Item = Value, Error = Response<()>> {
            let store = self.store.clone();
            let store_clone = store.clone();
            let parsed_id: Result<A::AccountId, ()> = A::AccountId::from_str(&id).map_err(|_| error!("Invalid id"));
            result(parsed_id)
                .map_err(|_| Response::builder().status(400).body(()).unwrap())
                .and_then(move |id| {
                    store.clone().get_account_from_http_auth(&authorization)
                        .and_then(move |account|
                            if account.id() == id {
                                Either::A(ok(json!(account)))
                            } else if account.is_admin() {
                                Either::B(store_clone.get_accounts(vec![id]).and_then(|accounts| Ok(json!(accounts[0].clone()))))
                            } else {
                                Either::A(err(()))
                            })
                        .map_err(move |_| {
                            debug!("No account found with auth: {}", authorization);
                            Response::builder().status(401).body(()).unwrap()
                        })
                })
        }

        // TODO should this be combined into the account record?
        #[get("/accounts/:id/balance")]
        #[content_type("application/json")]
        fn get_balance(&self, id: String, authorization: String) -> impl Future<Item = BalanceResponse, Error = Response<()>> {
            let store = self.store.clone();
            let store_clone = store.clone();
            let parsed_id: Result<A::AccountId, ()> = A::AccountId::from_str(&id).map_err(|_| error!("Invalid id"));
            result(parsed_id)
                .map_err(|_| Response::builder().status(400).body(()).unwrap())
                .and_then(move |id| {
                    store.clone().get_account_from_http_auth(&authorization)
                        .and_then(move |account|
                            if account.id() == id {
                                Either::A(ok(account))
                            } else if account.is_admin() {
                                Either::B(store_clone.get_accounts(vec![id]).and_then(|accounts| Ok(accounts[0].clone())))
                            } else {
                                Either::A(err(()))
                            })
                        .map_err(move |_| {
                            debug!("No account found with auth: {}", authorization);
                            Response::builder().status(401).body(()).unwrap()
                        })
                        .and_then(move |account| store.get_balance(account)
                        .and_then(|balance| Ok(BalanceResponse {
                            balance: balance.to_string(),
                        }))
                        .map_err(|_| Response::builder().status(404).body(()).unwrap()))
                })
        }

        #[put("/rates")]
        #[content_type("application/json")]
        fn post_rates(&self, body: Rates, authorization: String) -> impl Future<Item = Success, Error = Response<()>> {
            self.validate_admin(authorization)
                .and_then(move |store| store.set_rates(body.0)
                .and_then(|_| Ok(Success))
                .map_err(|err| {
                    error!("Error setting rates: {:?}", err);
                    Response::builder().status(500).body(()).unwrap()
                }))
        }

        #[get("/routes")]
        #[content_type("application/json")]
        fn get_routes(&self) -> impl Future<Item = Routes, Error = Response<()>> {
            ok(Routes(HashMap::from_iter(self.store.routing_table()
                .into_iter()
                .filter_map(|(address, account)| {
                    if let Ok(address) = str::from_utf8(address.as_ref()) {
                        Some((address.to_string(), account.to_string()))
                    } else {
                        None
                    }
                }))))
        }

        #[put("/routes/static")]
        #[content_type("application/json")]
        fn post_static_routes(&self, body: Routes, authorization: String) -> impl Future<Item = Success, Error = Response<()>> {
            self.validate_admin(authorization)
                .and_then(move |store| {
                    let mut routes: HashMap<String, A::AccountId> = HashMap::with_capacity(body.0.len());
                    for (prefix, account_id) in body.0 {
                        if let Ok(account_id) = A::AccountId::from_str(account_id.as_str()) {
                            routes.insert(prefix, account_id);
                        } else {
                            return Err(Response::builder().status(400).body(()).unwrap());
                        }
                    }
                    Ok((store, routes))
                })
                .and_then(|(store, routes)| {
                    store.set_static_routes(routes)
                    .and_then(|_| Ok(Success))
                        .map_err(|err| {
                            error!("Error setting static routes: {:?}", err);
                            Response::builder().status(500).body(()).unwrap()
                        })
            })
        }

        #[put("/routes/static/:prefix")]
        #[content_type("application/json")]
        fn post_static_route(&self, prefix: String, body: String, authorization: String) -> impl Future<Item = Success, Error = Response<()>> {
            self.validate_admin(authorization)
                .and_then(move |store| {
                    if let Ok(account_id) = A::AccountId::from_str(body.as_str()) {
                        Ok((store, account_id))
                    } else {
                        Err(Response::builder().status(400).body(()).unwrap())
                    }
                })
                .and_then(move |(store, account_id)| {
                    store.set_static_route(prefix, account_id)
                    .and_then(|_| Ok(Success))
                        .map_err(|err| {
                            error!("Error setting static route: {:?}", err);
                            Response::builder().status(500).body(()).unwrap()
                        })
                })
        }

        #[post("/pay")]
        #[content_type("application/json")]
        // TODO add a version that lets you specify the destination amount instead
        fn post_pay(&self, body: SpspPayRequest, authorization: String) -> impl Future<Item = SpspPayResponse, Error = Response<String>> {
            let service = self.incoming_handler.clone();
            self.store.get_account_from_http_auth(&authorization)
                .map_err(|_| Response::builder().status(401).body("Unauthorized".to_string()).unwrap())
                .and_then(move |account| {
                    pay(service, account, &body.receiver, body.source_amount)
                        .and_then(|amount_delivered| Ok(SpspPayResponse {
                                amount_delivered,
                            }))
                        .map_err(|err| {
                            error!("Error sending SPSP payment: {:?}", err);
                            // TODO give a different error message depending on what type of error it is
                            Response::builder().status(500).body(format!("Error sending SPSP payment: {:?}", err)).unwrap()
                        })
                })
        }

        #[post("/ilp")]
        // TODO make sure taking the body as a Vec (instead of Bytes) doesn't cause a copy
        // for some reason, it complains that Extract isn't implemented for Bytes even though tower-web says it is
        fn post_ilp(&self, body: Vec<u8>, authorization: String) -> impl Future<Item = Response<Body>, Error = Error> {
            let request = Request::builder()
                .header("Authorization", authorization)
                .body(Body::from(body))
                .unwrap();
            HttpServerService::new(self.incoming_handler.clone(), self.store.clone()).handle_http_request(request)
        }

        #[get("/spsp/:id")]
        fn get_spsp(&self, id: String) -> impl Future<Item = Response<Body>, Error = Response<()>> {
            let server_secret = self.server_secret.clone();
            let store = self.store.clone();
            let id: Result<A::AccountId, ()> = A::AccountId::from_str(&id).map_err(|_| error!("Invalid id: {}", id));
            result(id)
                .map_err(|_| Response::builder().status(400).body(()).unwrap())
                .and_then(move |id| store.get_accounts(vec![id])
                .map_err(move |_| {
                    error!("Account not found: {}", id);
                    Response::builder().status(404).body(()).unwrap()
                }))
                .and_then(move |accounts| {
                    let ilp_address = Bytes::from(accounts[0].client_address());
                    // TODO return the response without instantiating an SpspResponder (use a simple fn)
                    Ok(SpspResponder::new(ilp_address, server_secret)
                        .generate_http_response())
                    })
        }

        // TODO resolve payment pointers with subdomains to the correct account
        // also give accounts aliases to use in the payment pointer instead of the ids
        #[get("/.well-known/pay")]
        fn get_well_known(&self) -> impl Future<Item = Response<Body>, Error = Response<()>> {
            let default_account = A::AccountId::default();
            let server_secret = self.server_secret.clone();
            self.store.get_accounts(vec![default_account])
            .map_err(move |_| {
                error!("Account not found: {}", default_account);
                Response::builder().status(404).body(()).unwrap()
            })
            .and_then(move |accounts| {
                let ilp_address = Bytes::from(accounts[0].client_address());
                Ok(SpspResponder::new(ilp_address, server_secret)
                    .generate_http_response())
                })
        }

        // TODO add quoting via SPSP/STREAM
    }
}
