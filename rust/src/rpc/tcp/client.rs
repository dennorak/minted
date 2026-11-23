//! Outbound connection helpers. The connection itself is always an `RpcPeer`.
//!
//! `RpcClient` is only a type alias; generated typed clients can hold either
//! name, and both can receive calls when given a local dispatcher.

use std::sync::Arc;

use tokio::net::{TcpStream, ToSocketAddrs};

use super::super::{RpcCallResult, RpcDispatcher, RpcPeer, RpcPeerConfig};

impl RpcPeer {
    /// Initiates a TCP connection, with no local method registry.
    pub async fn connect_tcp<A>(address: A) -> RpcCallResult<Self>
    where
        A: ToSocketAddrs,
    {
        Self::connect_tcp_with_config(address, RpcPeerConfig::default()).await
    }

    /// Initiates a TCP connection with the given configuration, with no local method registry.
    pub async fn connect_tcp_with_config<A>(
        address: A,
        config: RpcPeerConfig,
    ) -> RpcCallResult<Self>
    where
        A: ToSocketAddrs,
    {
        let stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;
        Ok(Self::from_stream_with_config(stream, None, config))
    }

    /// Initiates a connection that can also serve calls from the remote peer.
    pub async fn connect_tcp_with_dispatcher<A>(
        address: A,
        dispatcher: Arc<dyn RpcDispatcher>,
    ) -> RpcCallResult<Self>
    where
        A: ToSocketAddrs,
    {
        Self::connect_tcp_with_dispatcher_and_config(address, dispatcher, RpcPeerConfig::default())
            .await
    }

    /// Initiates a connection that can also serve calls from the remote peer, with the given configuration.
    pub async fn connect_tcp_with_dispatcher_and_config<A>(
        address: A,
        dispatcher: Arc<dyn RpcDispatcher>,
        config: RpcPeerConfig,
    ) -> RpcCallResult<Self>
    where
        A: ToSocketAddrs,
    {
        let stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;
        Ok(Self::from_stream_with_config(
            stream,
            Some(dispatcher),
            config,
        ))
    }
}
