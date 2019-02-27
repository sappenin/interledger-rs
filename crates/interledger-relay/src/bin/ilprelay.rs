use futures::prelude::*;
use hyper::Uri;

use interledger_relay::{AuthToken, NextHop, Route};
use interledger_relay::app::ConnectorBuilder;

//struct Config (maybe in another module)

fn main() {
    // TODO config from json
    let connector = ConnectorBuilder {
        ilp_addr: b"example.alice".to_vec(),
        auth_tokens: vec![
            AuthToken::new(b"secret".to_vec()),
        ],
        routes: vec![
            Route::new(
                b"".to_vec(),
                NextHop::new(
                    "http://127.0.0.1:3002/ilp".parse::<Uri>().unwrap(),
                    None,
                ),
            ),
        ],
    }.build();

    hyper::rt::run({
        hyper::Server::bind(&([127, 0, 0, 1], 3001).into())
            // NOTE: `hyper::Error` is a placeholder.. The "never" type would
            // be better once it's stable.
            .serve(move || -> Result<_, hyper::Error> {
                Ok(connector.clone())
            })
            .map_err(|error| {
                eprintln!("server error: {}", error)
            })
    });
}
