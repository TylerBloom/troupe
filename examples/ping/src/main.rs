use std::time::Duration;

use ping::Ping;
use troupe::ActorBuilder;

mod ping;

#[tokio::main]
async fn main() {
    // Construct the builder for the ping "server"
    let builder = ActorBuilder::new(Ping::default());
    // We can get a client before launching the actor process.
    let client = builder.sink_client();
    // We can send messages to the client but they will not be processed until we launch the actor.
    client.send(());
    let tracker = client.track(());
    tokio::spawn(async move {
        let resp = tracker.await;
        println!("Got response from ping server: {resp}");
        // Messages are processed in order, so we know that the second message will yield 1
        assert_eq!(resp, 1);
    });
    tokio::spawn(async move {
        let resp = client.track(()).await;
        println!("Got response from ping server: {resp}");
        assert_eq!(resp, 2);
    });
    // Spawn the actor process, which returns a client
    let client = builder.launch_sink();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let resp = client.track(()).await;
    println!("Got response from ping server: {resp}");
    assert_eq!(resp, 3);
}
