use std::time::Duration;

use ping::Ping;
use troupe::ActorBuilder;

mod ping;

#[tokio::main]
async fn main() {
    // Spawn the actor process. `SinkActor`'s config is `()`, so the builder needs nothing else
    // before launching. Launching hands back the kind's client, a `SinkClient`.
    let client = ActorBuilder::new(Ping::default()).launch();
    // The actor processes messages in the order that they are sent, so we can queue up a series of
    // messages and know what each response will be.
    client.send(());
    let first = client.track(());
    let second = client.track(());
    // Clients are cheap to clone, so each part of your application can hold its own.
    let other_client = client.clone();
    tokio::spawn(async move {
        // Awaiting a `Tracker` yields an `Option` because the actor may have shut down before it
        // could respond.
        let resp = first.await.unwrap();
        println!("Got response from ping server: {resp}");
        // The first `send` bumped the counter, so this message sees a count of 1.
        assert_eq!(resp, 1);
    });
    tokio::spawn(async move {
        let resp = second.await.unwrap();
        println!("Got response from ping server: {resp}");
        assert_eq!(resp, 2);
        drop(other_client);
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    let resp = client.track(()).await.unwrap();
    println!("Got response from ping server: {resp}");
    assert_eq!(resp, 3);
}
