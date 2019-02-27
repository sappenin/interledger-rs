//! Test helpers, mocks, and fixtures.

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use futures::future::FutureResult;
use futures::prelude::*;
use hyper::Uri;
use lazy_static::lazy_static;
use tokio::runtime::Runtime;
use tokio::timer::Delay;

use super::{NextHop, Route, Service};

const EXPIRES_IN: Duration = Duration::from_secs(20);

pub static RECEIVER_ORIGIN: &'static str = "http://127.0.0.1:3001";
static RECEIVER_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 3001);

lazy_static! {
    pub static ref PREPARE: ilp::Prepare = ilp::PrepareBuilder {
        amount: 123,
        expires_at: round_time(SystemTime::now() + EXPIRES_IN),
        execution_condition: b"\
            \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
            \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
        ",
        destination: b"test.bob.1234",
        data: b"prepare data",
    }.build();

    pub static ref FULFILL: ilp::Fulfill = ilp::FulfillBuilder {
        fulfillment: b"\
            \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
            \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
        ",
        data: b"fulfill data",
    }.build();

    pub static ref REJECT: ilp::Reject = ilp::RejectBuilder {
        code: ilp::ErrorCode::F99_APPLICATION_ERROR,
        message: b"Some error",
        triggered_by: b"example.connector",
        data: b"reject data",
    }.build();

    pub static ref ROUTES: Vec<Route> = vec![
        Route::new(
            b"test.alice.".to_vec(),
            NextHop::new(
                format!("{}/alice", RECEIVER_ORIGIN).parse::<Uri>().unwrap(),
                Some(b"alice_auth".to_vec()),
            ),
        ),
        Route::new(
            b"test.bob.".to_vec(),
            NextHop::new(
                format!("{}/bob", RECEIVER_ORIGIN).parse::<Uri>().unwrap(),
                Some(b"bob_auth".to_vec()),
            ),
        ),
        Route::new(
            b"".to_vec(),
            NextHop::new(
                format!("{}/default", RECEIVER_ORIGIN).parse::<Uri>().unwrap(),
                Some(b"default_auth".to_vec()),
            ),
        ),
    ];
}

fn round_time(mut time: SystemTime) -> SystemTime {
    let since_epoch = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    time -= Duration::from_nanos(since_epoch.subsec_nanos() as u64);
    time
}

pub type IlpResult = Result<ilp::Fulfill, ilp::Reject>;

#[derive(Clone, Debug)]
pub struct MockService {
    prepares: Arc<RwLock<Vec<ilp::Prepare>>>,
    response: Arc<IlpResult>,
}

impl MockService {
    pub fn new(response: IlpResult) -> Self {
        MockService {
            prepares: Arc::new(RwLock::new(Vec::new())),
            response: Arc::new(response),
        }
    }

    pub fn prepares(&self) -> Vec<ilp::Prepare> {
        self.prepares.read()
            .unwrap()
            .clone()
    }
}

impl Service for MockService {
    type Future = FutureResult<ilp::Fulfill, ilp::Reject>;
    fn call(self, prepare: ilp::Prepare) -> Self::Future {
        self.prepares
            .write()
            .unwrap()
            .push(prepare);
        self.response.as_ref().clone().into_future()
    }
}

#[derive(Clone, Debug)]
pub struct PanicService;

impl Service for PanicService {
    type Future = Box<dyn Future<
        Item = ilp::Fulfill,
        Error = ilp::Reject,
    > + Send + 'static>;
    fn call(self, _prepare: ilp::Prepare) -> Self::Future {
        panic!("PanicService received prepare");
    }
}

lazy_static! {
    static ref SERVER_MUTEX: Mutex<()> = Mutex::new(());
}

#[derive(Clone)]
pub struct MockServer {
    test_request: fn(&hyper::Request<hyper::Body>),
    test_body: fn(hyper::Chunk),
    /// An error variant indicates the response should abort the connection.
    make_response: Result<
        fn() -> hyper::Response<hyper::Body>,
        (),
    >,
    delay: Option<Duration>,
}

impl MockServer {
    pub fn new() -> Self {
        MockServer {
            test_request: |_req| {},
            test_body: |_body| {},
            make_response: Ok(|| { panic!("missing make_response") }),
            delay: None,
        }
    }

    /// Test the incoming request.
    pub fn test_request(
        mut self,
        test: fn(&hyper::Request<hyper::Body>),
    ) -> Self {
        self.test_request = test;
        self
    }

    /// Test the incoming request body.
    pub fn test_body(mut self, test: fn(hyper::Chunk)) -> Self {
        self.test_body = test;
        self
    }

    pub fn with_response(
        mut self,
        make_response: fn() -> hyper::Response<hyper::Body>,
    ) -> Self {
        self.make_response = Ok(make_response);
        self
    }

    /// Abort the connection after received a request, before sending a response.
    pub fn with_abort(mut self) -> Self {
        self.make_response = Err(());
        self
    }

    /// Time to wait before responding.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn run<Test>(self, run: Test)
    where
        Test: 'static + Future<Item = ()> + Send,
    {
        // Ensure that parallel tests don't fight over the server port.
        let _guard = SERVER_MUTEX.lock().unwrap();
        let delay = self.delay.unwrap_or(Duration::from_secs(0));

        let receiver = hyper::Server::bind(&RECEIVER_ADDR.into())
            .serve(move || {
                // The cloning is a bit of a mess, but seems to be necessary
                // to untangle the closure lifetimes.
                let mock = self.clone();
                hyper::service::service_fn(move |req| {
                    let mock = mock.clone();
                    (mock.test_request)(&req);
                    Delay::new(Instant::now() + delay)
                        .then(|_| { req.into_body().concat2() })
                        .then(move |body_result| {
                            (mock.test_body)(body_result.unwrap());
                            match mock.make_response {
                                Ok(make_resp) => Ok(make_resp()),
                                Err(_) => Err("abort!"),
                            }
                        })
                })
            })
            .with_graceful_shutdown(run)
            .map_err(|err| { panic!("server error: {}", err) });

        let mut rt = Runtime::new().unwrap();
        rt.block_on(receiver).unwrap();
        rt.shutdown_now()
            .wait()
            .unwrap()
    }
}
