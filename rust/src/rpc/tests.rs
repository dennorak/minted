//! In-process transport tests. TestCodec moves Values through a shared table;
//! the real frame parser and envelope conversion are still exercised.

extern crate self as minted;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::io::duplex;

use super::{
    RpcContext, RpcDispatcher, RpcError, RpcPeer, RpcResult,
    router::{RoutedDispatcher, RpcHandler, RpcRouter},
};
use crate::Value;

struct Ping;

#[async_trait]
impl RpcHandler<()> for Ping {
    fn name(&self) -> &'static str {
        "ping"
    }

    async fn handle(&self, _: &RpcContext, _: &(), _: &Value) -> RpcResult {
        Ok(Value::String("pong".into()))
    }
}

struct Callback;

#[async_trait]
impl RpcHandler<()> for Callback {
    fn name(&self) -> &'static str {
        "callback"
    }

    async fn handle(&self, context: &RpcContext, _: &(), _: &Value) -> RpcResult {
        // Requires the receiving peer to keep its read loop running while
        // this handler waits for a response to the reverse call.
        context
            .call("ping", Value::Null)
            .await
            .map_err(|_| RpcError::internal())
    }
}

struct Count(Arc<AtomicUsize>);

#[async_trait]
impl RpcHandler<()> for Count {
    fn name(&self) -> &'static str {
        "count"
    }

    async fn handle(&self, _: &RpcContext, _: &(), _: &Value) -> RpcResult {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Value::Null)
    }
}

fn dispatcher(router: RpcRouter<()>) -> Arc<dyn RpcDispatcher> {
    Arc::new(RoutedDispatcher::new((), router))
}

#[tokio::test]
async fn nested_reverse_call_completes() {
    let (left_stream, right_stream) = duplex(4096);

    let mut left_router = RpcRouter::new();
    left_router.register(Ping);
    let left = RpcPeer::from_stream_with_dispatcher(left_stream, dispatcher(left_router));

    let mut right_router = RpcRouter::new();
    right_router.register(Callback);
    let right = RpcPeer::from_stream_with_dispatcher(right_stream, dispatcher(right_router));

    let value = tokio::time::timeout(Duration::from_secs(2), left.call("callback", Value::Null))
        .await
        .expect("reverse callback deadlocked")
        .unwrap();
    assert_eq!(value, Value::String("pong".into()));

    left.close();
    right.close();
}

#[tokio::test]
async fn both_peers_can_use_the_same_request_id() {
    let (left_stream, right_stream) = duplex(4096);
    let mut left_router = RpcRouter::new();
    left_router.register(Ping);
    let mut right_router = RpcRouter::new();
    right_router.register(Ping);
    let left = RpcPeer::from_stream_with_dispatcher(left_stream, dispatcher(left_router));
    let right = RpcPeer::from_stream_with_dispatcher(right_stream, dispatcher(right_router));

    let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            left.call("ping", Value::Null),
            right.call("ping", Value::Null),
        )
    })
    .await
    .expect("bidirectional requests deadlocked");
    assert_eq!(a.unwrap(), Value::String("pong".into()));
    assert_eq!(b.unwrap(), Value::String("pong".into()));

    left.close();
    right.close();
}

#[tokio::test]
async fn notification_is_dispatched_and_observable() {
    let (left_stream, right_stream) = duplex(4096);
    let left = RpcPeer::from_stream(left_stream);
    let count = Arc::new(AtomicUsize::new(0));
    let mut router = RpcRouter::new();
    router.register(Count(Arc::clone(&count)));
    let right = RpcPeer::from_stream_with_dispatcher(right_stream, dispatcher(router));
    let mut received = right.subscribe_notifications();

    left.notify("count", Value::Uint(3)).await.unwrap();
    let note = tokio::time::timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.method, "count");
    assert_eq!(note.params, Value::Uint(3));

    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("notification handler was not called");

    left.close();
    right.close();
}

#[crate::rpc::rpc(client, server)]
async fn echo(value: String, times: u8) -> RpcResult<String> {
    Ok(value.repeat(times as usize))
}

#[tokio::test]
async fn generated_client_and_server_can_call_each_other() {
    use echo::Client as _;

    let (left_stream, right_stream) = duplex(4096);

    // Register the generated server handler.
    let router = RpcRouter::<()>::new().route(echo::register);
    let dispatcher = Arc::new(RoutedDispatcher::new((), router));

    // // Both endpoints are RpcPeers.
    let left = RpcPeer::from_stream(left_stream);
    let right = RpcPeer::from_stream_with_dispatcher(right_stream, dispatcher);

    // Call the generated typed client function.
    let value = tokio::time::timeout(Duration::from_secs(2), left.echo("hi".to_string(), 3))
        .await
        .expect("generated RPC call deadlocked")
        .expect("RPC call failed");

    assert_eq!(value, "hihihi".to_string());

    left.close();
    right.close();
}
