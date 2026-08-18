use cache::Cache;
use troupe::ActorBuilder;

mod cache;

#[tokio::main]
async fn main() {
    let state: Cache<i32, &'static str> = Cache::default();
    // `SinkActor`'s config is `()`, so the builder can be launched directly. Launching hands back
    // the kind's client, a `SinkClient`.
    let client = ActorBuilder::new(state).launch();
    client.send((1, "one"));
    // Awaiting a `Tracker` yields an `Option` because the actor may have shut down before it could
    // respond. Here, the actor's response is itself an `Option`, hence the `flatten`.
    let val = client.track(1).await.flatten().unwrap();
    assert_eq!(val, "one");
    let answer = client
        .track((1, |s: &&str| s.to_string()))
        .await
        .flatten()
        .unwrap_or_default();
    assert_eq!(answer, "one");
    let answer = client
        .track((2, |s: &&str| s.to_string()))
        .await
        .flatten()
        .unwrap_or_default();
    assert_eq!(answer, "");
    client.send(1);
    let answer = client
        .track((1, |s: &&str| s.to_string()))
        .await
        .flatten()
        .unwrap_or_default();
    assert_eq!(answer, "");
}
