//! Inbound connection helpers. Accepted connections are the same `RpcPeer`
//! as outbound ones, so the server can call methods on connected clients.

use std::{future::Future, sync::Arc};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, ToSocketAddrs},
    sync::Semaphore,
    task::JoinSet,
};

use super::super::{
    RoutedDispatcher, RpcCallResult, RpcDispatcher, RpcPeer, RpcPeerConfig, RpcRouter,
};

/// Server configuration. The `max_connections` must be positive.
#[derive(Debug, Clone, Copy)]
pub struct RpcServerConfig {
    /// Maximum number of concurrent connections. New connections are accepted
    pub max_connections: usize,
    /// Configuration for each peer. This is cloned for each connection.
    pub peer: RpcPeerConfig,
}

impl Default for RpcServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 128,
            peer: RpcPeerConfig::default(),
        }
    }
}

// Closing the listener's connection task must close its peer, even if an
// application has cloned and retained the handle in a registry.
struct ClosePeerOnDrop(RpcPeer);

impl Drop for ClosePeerOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// RPC server. Accepts connections and dispatches calls to the given router.
pub struct RpcServer<S> {
    state: Arc<S>,
    router: Arc<RpcRouter<S>>,
    config: RpcServerConfig,
}

impl<S> Clone for RpcServer<S> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            router: Arc::clone(&self.router),
            config: self.config,
        }
    }
}

impl<S> RpcServer<S> {
    /// Create a new server with the given state and router. The state is shared
    /// with all connections, and the router is used to dispatch incoming calls.
    pub fn new(state: S, router: RpcRouter<S>) -> Self {
        Self::from_shared(Arc::new(state), Arc::new(router))
    }

    /// Create a new server with the given shared state and router. The state is shared with all connections, and the router is used to dispatch incoming calls.
    pub fn from_shared(state: Arc<S>, router: Arc<RpcRouter<S>>) -> Self {
        Self {
            state,
            router,
            config: RpcServerConfig::default(),
        }
    }

    /// Set the server configuration. The `max_connections` must be positive.
    pub fn with_config(mut self, config: RpcServerConfig) -> Self {
        assert!(
            config.max_connections > 0,
            "max_connections must be positive"
        );
        self.config = config;
        self
    }
}

impl<S: Send + Sync + 'static> RpcServer<S> {
    /// Attach an already-connected stream and return the peer handle. This is
    /// useful for manually accepted streams or initiating server-side calls.
    pub fn attach<T>(&self, stream: T) -> RpcPeer
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.attach_shared(stream)
    }

    fn attach_shared<T>(&self, stream: T) -> RpcPeer
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let dispatcher: Arc<dyn RpcDispatcher> = Arc::new(RoutedDispatcher::from_shared(
            Arc::clone(&self.state),
            Arc::clone(&self.router),
        ));
        RpcPeer::from_stream_with_shared_codec(stream, Some(dispatcher), self.config.peer)
    }

    /// Run a single connection until it closes. To make reverse calls,
    /// use `attach()` instead and keep the resulting peer handle.
    pub async fn serve_stream<T>(&self, stream: T) -> RpcCallResult<()>
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let peer = self.attach(stream);
        peer.wait_closed().await;
        Ok(())
    }

    /// Run a TCP server on the given address. The server will accept connections
    /// until the returned future is dropped. Each connection is handled in a
    /// separate task, and the `on_connect` callback is called for each new peer.
    pub async fn serve_tcp<A, C>(&self, address: A) -> RpcCallResult<()>
    where
        A: ToSocketAddrs,
    {
        let listener = TcpListener::bind(address).await?;
        self.serve_listener(listener).await
    }

    /// Run a TCP server on the given listener. The server will accept connections
    /// until the returned future is dropped. Each connection is handled in a
    /// separate task, and the `on_connect` callback is called for each new peer.
    pub async fn serve_listener(&self, listener: TcpListener) -> RpcCallResult<()> {
        self.serve_listener_until(listener, std::future::pending::<()>())
            .await
    }

    /// Run a TCP server on the given listener until the `shutdown` future resolves.
    /// Each connection is handled in a separate task, and the `on_connect`
    /// callback is called for each new peer.
    pub async fn serve_listener_until<Stop>(
        &self,
        listener: TcpListener,
        shutdown: Stop,
    ) -> RpcCallResult<()>
    where
        Stop: Future<Output = ()>,
    {
        self.serve_listener_until_with(listener, |_| async {}, shutdown)
            .await
    }

    /// `on_connect` is given a cloneable peer handle. It can initiate reverse
    /// calls, or hand the handle to an external connection registry.
    ///
    /// A connection stays alive even if this callback finishes. On server
    /// shutdown, all connection tasks and callbacks are cancelled.
    pub async fn serve_listener_until_with<OnConnect, Connected, Stop>(
        &self,
        listener: TcpListener,
        on_connect: OnConnect,
        shutdown: Stop,
    ) -> RpcCallResult<()>
    where
        OnConnect: Fn(RpcPeer) -> Connected + Send + Sync + 'static,
        Connected: Future<Output = ()> + Send + 'static,
        Stop: Future<Output = ()>,
    {
        let on_connect = Arc::new(on_connect);
        let slots = Arc::new(Semaphore::new(self.config.max_connections));
        let mut connections = JoinSet::<()>::new();
        tokio::pin!(shutdown);

        loop {
            // Acquire *before* accept, so at most max_connections are active.
            let permit = tokio::select! {
                _ = &mut shutdown => break,
                permit = Arc::clone(&slots).acquire_owned() => {
                    permit.expect("RPC connection semaphore closed")
                }
            };

            let accepted = tokio::select! {
                _ = &mut shutdown => { drop(permit); break; }
                accepted = listener.accept() => accepted,
            };
            let (stream, _) = accepted?;
            let _ = stream.set_nodelay(true);
            let server = self.clone();
            let on_connect = Arc::clone(&on_connect);
            connections.spawn(async move {
                let _permit = permit;
                let peer = server.attach_shared(stream);
                let _close_on_exit = ClosePeerOnDrop(peer.clone());
                let callback = on_connect(peer.clone());
                tokio::pin!(callback);

                // The callback may complete immediately or live for the
                // connection's lifetime. Neither case should close the peer.
                tokio::select! {
                    _ = peer.wait_closed() => {},
                    _ = &mut callback => peer.wait_closed().await,
                }
                peer.close();
            });

            // Retire finished connection tasks without affecting socket reads.
            while let Some(finished) = connections.try_join_next() {
                if let Err(error) = finished {
                    eprintln!("Minted RPC connection task failed: {error}");
                }
            }
        }

        connections.abort_all();
        while connections.join_next().await.is_some() {}
        Ok(())
    }
}
