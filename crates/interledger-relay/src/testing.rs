//! Test helpers, mocks, and fixtures.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::future::FutureResult;
use futures::prelude::*;
use lazy_static::lazy_static;

use super::Service;

lazy_static! {
    pub static ref PREPARE: ilp::Prepare = ilp::PrepareBuilder {
        amount: 123,
        expires_at: round_time(SystemTime::now()),
        execution_condition: b"\
            \x11\x7b\x43\x4f\x1a\x54\xe9\x04\x4f\x4f\x54\x92\x3b\x2c\xff\x9e\
            \x4a\x6d\x42\x0a\xe2\x81\xd5\x02\x5d\x7b\xb0\x40\xc4\xb4\xc0\x4a\
        ",
        destination: b"test.bob",
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
    prepares: Arc<RefCell<Vec<ilp::Prepare>>>,
    response: IlpResult,
}

impl MockService {
    pub fn new(response: IlpResult) -> Self {
        MockService {
            prepares: Arc::new(RefCell::new(Vec::new())),
            response,
        }
    }

    pub fn prepares(&self) -> Vec<ilp::Prepare> {
        self.prepares.borrow().clone()
    }
}

impl Service for MockService {
    type Future = FutureResult<ilp::Fulfill, ilp::Reject>;
    fn call(self, prepare: ilp::Prepare) -> Self::Future {
        self.prepares.borrow_mut().push(prepare);
        self.response.clone().into_future()
    }
}
