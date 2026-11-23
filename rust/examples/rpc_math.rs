#[cfg(feature = "rpc")]
use std::sync::Arc;

#[cfg(feature = "rpc")]
use serde::{Serialize, Deserialize};
#[cfg(feature = "rpc")]
use minted::prelude::*;
#[cfg(feature = "rpc")]
use tokio::io::{AsyncRead, AsyncWrite};

#[cfg(feature = "rpc")]
#[derive(Serialize, Deserialize)]
enum OtherMathRequestKind {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[cfg(feature = "rpc")]
#[rpc(client, server, nested_under = "math")]
async fn add(a: i32, b: i32) -> RpcResult<i32> {
    Ok(a + b)
}

#[cfg(feature = "rpc")]
#[rpc(client, server, nested_under = "math")]
async fn other(kind: OtherMathRequestKind, a: i32, b: i32) -> RpcResult<i32> {
    match kind {
        OtherMathRequestKind::Add => Ok(a + b),
        OtherMathRequestKind::Subtract => Ok(a - b),
        OtherMathRequestKind::Multiply => Ok(a * b),
        OtherMathRequestKind::Divide => {
            if b == 0 {
                // application error must be returned with a code >= 10000
                Err(RpcError::application(10000, "Division by zero"))
            } else {
                Ok(a / b)
            }
        }
    }
}

#[cfg(not(feature = "rpc"))]
fn main() {
    eprintln!("This example requires the 'rpc' feature to be enabled.");
    std::process::exit(1);
}

#[cfg(feature = "rpc")]
#[tokio::main]
async fn main() {
    // check if first arg is "tcp" or "memory"
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            #[cfg(feature = "rpc-tcp")]
            "tcp" => {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:12100")
                    .await
                    .expect("Failed to bind TCP listener");

                println!("Listening on 127.0.0.1:12100");

                let right = tokio::net::TcpStream::connect("127.0.0.1:12100")
                    .await
                    .expect("Failed to connect to TCP listener");

                let (left, _) = listener
                    .accept()
                    .await
                    .expect("Failed to accept TCP connection");

                handle_with_left_right(left, right).await;
            }
            "memory" => {
                let (left, right) = tokio::io::duplex(4096);
                handle_with_left_right(left, right).await;
            }
            _ => {
                eprintln!("Unknown argument: {}", arg);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Usage: {} [tcp|memory]", std::env::args().next().unwrap());
        std::process::exit(1);
    };
}

#[cfg(feature = "rpc")]
async fn handle_with_left_right<L, R>(left: L, right: R)
where
    L: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    R: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Register the generated server handler.
    let router = RpcRouter::<()>::new().route(add::register).route(other::register);
    let dispatcher = Arc::new(RoutedDispatcher::new((), router));

    // Both endpoints are RpcPeers.
    let left = RpcPeer::from_stream_with_dispatcher(left, dispatcher.clone());
    let _right = RpcPeer::from_stream_with_dispatcher(right, dispatcher);

    // Call the math method from the left peer.
    let result = left.add(2, 3).await.expect("RPC call failed");
    println!("2 + 3 = {}", result);

    let result = left.other(OtherMathRequestKind::Multiply, 4, 5).await.expect("RPC call failed");
    println!("4 * 5 = {}", result);

    let result = left.other(OtherMathRequestKind::Divide, 10, 0).await;
    match result {
        Ok(value) => println!("10 / 0 = {}", value),
        Err(e) => println!("(should) Error: {}", e),
    }

    let result = left.other(OtherMathRequestKind::Divide, 10, 2).await.expect("RPC call failed");
    println!("10 / 2 = {}", result);
}