use cache::Cache;
use troupe::ActorBuilder;

mod cache;

#[tokio::main]
async fn main() {
    let state: Cache<i32, &'static str> = Cache::default();
    let client = ActorBuilder::new(state).launch_sink();
    client.send((1, "one"));
    let val = client.track(1).await.unwrap();
    assert_eq!(val, "one");
    let answer = client.track((1, |s: &&str| s.to_string())).await.unwrap_or_default();
    assert_eq!(answer, "one");
    let answer = client.track((2, |s: &&str| s.to_string())).await.unwrap_or_default();
    assert_eq!(answer, "");
    client.send(1);
    let answer = client.track((1, |s: &&str| s.to_string())).await.unwrap_or_default();
    assert_eq!(answer, "");
}
