use futures::Future;
use hyper::{Server, Uri};
//use hyper::service::Service;

use ilp_connector_relay::{Receiver, Relay, Route};

fn main() {
    println!("ilp relay start");

    // TODO filter by path, method, etc
    //let relay = Arc::new(Relay::new(
    let relay = Relay::new(
        b"example.alice".to_vec(),
        vec![
            Route::new(b"".to_vec(), "http://127.0.0.1:3002/ilp".parse::<Uri>().unwrap()),
        ],
        // TODO config
    );
    //));

    let receiver = Receiver::new(relay);

    // TODO config, env
    let bind_addr = ([127, 0, 0, 1], 3001).into();
    let server = Server::bind(&bind_addr)
        //.serve(hyper::service::make_service_fn(|socket: &TcpStream| Ok(&relay)))
        //.serve(|| -> FutureResult<Arc<Relay>, !> {
        .serve(move || {
            //let relay = relay.clone();
            ////let relay = Arc::clone(&relay);
            hyper::service::service_fn(move |req| {
                relay.call(req)
            })
        })
        .map_err(|error| {
            eprintln!("server error: {}", error)
        });

    println!("Listening on http://{}", bind_addr);
    hyper::rt::run(server);
}
