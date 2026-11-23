//! A single bidirectional RPC connection.
//!
//! Both endpoints may initiate calls and service incoming requests. The
//! transport has one reader and one writer; handlers execute independently
//! so a handler can call back into the remote peer without deadlocking.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::{
        broadcast, mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore,
    },
    task::JoinSet,
};

use crate::Value;

use super::{
    codec::{read_message, write_message, DEFAULT_MAX_FRAME_BYTES},
    RpcCallError, RpcCallResult, RpcError, RpcMessage, RpcResult,
};

/// The connection-independent entry point for incoming methods.
///
/// `RoutedDispatcher<S>` adapts an existing `RpcRouter<S>` to this trait.
/// The generated server adapters can eventually implement this too.
#[async_trait]
pub trait RpcDispatcher: Send + Sync {
    /// Dispatch an incoming request or notification. The `context` can be used
    /// to initiate reverse calls on the same connection. The `method` is the
    /// requested method name, and `params` is the Minted value sent by the
    /// remote peer. The result is sent back to the caller if this was a request.
    async fn dispatch(
        &self,
        context: &RpcContext,
        method: &str,
        params: &Value,
    ) -> RpcResult;
}

/// Configuration for a single RPC connection. The default is suitable for
/// most applications.
#[derive(Debug, Clone, Copy)]
pub struct RpcPeerConfig {
    /// Maximum size of a frame, excluding its four-byte length prefix.
    pub max_frame_bytes: usize,
    /// Bounded queue shared by responses, requests and notifications.
    pub outbound_capacity: usize,
    /// Limit on simultaneous incoming request/notification handlers.
    pub max_in_flight: usize,
    /// Limit on locally initiated requests awaiting a response.
    pub max_pending_requests: usize,
    /// Capacity of the optional notification observer channel.
    pub notification_capacity: usize,
    /// Capacity of the local diagnostic event channel.
    pub event_capacity: usize,
}

impl Default for RpcPeerConfig {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            outbound_capacity: 128,
            max_in_flight: 64,
            max_pending_requests: 1024,
            notification_capacity: 128,
            event_capacity: 128,
        }
    }
}

impl RpcPeerConfig {
    fn validate(self) {
        assert!(self.max_frame_bytes > 0, "max_frame_bytes must be positive");
        assert!(self.outbound_capacity > 0, "outbound_capacity must be positive");
        assert!(self.max_in_flight > 0, "max_in_flight must be positive");
        assert!(self.max_pending_requests > 0, "max_pending_requests must be positive");
        assert!(self.notification_capacity > 0, "notification_capacity must be positive");
        assert!(self.event_capacity > 0, "event_capacity must be positive");
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcNotification {
    pub method: String,
    pub params: Value,
}

/// Local events are never sent on the wire. Broadcast receivers may lag.
#[derive(Debug, Clone)]
pub enum RpcPeerEvent {
    NotificationFailed { method: String, error: RpcError },
    NotificationDropped { method: String },
    ConnectionError(String),
}

/// A weak per-request connection handle. Does not keep the connection alive
/// and can safely be passed to handlers for reverse calls.
#[derive(Clone)]
pub struct RpcContext {
    inner: Weak<PeerInner>,
}

impl RpcContext {
    fn new(inner: &Arc<PeerInner>) -> Self {
        Self { inner: Arc::downgrade(inner) }
    }

    /// Obtain a cheap, owned peer handle, if the connection is still open.
    pub fn peer(&self) -> RpcCallResult<RpcPeer> {
        let inner = self.inner.upgrade().ok_or(RpcCallError::ConnectionClosed)?;
        if inner.closed.load(Ordering::Acquire) {
            return Err(RpcCallError::ConnectionClosed);
        }
        inner.owner_count.fetch_add(1, Ordering::Relaxed);
        Ok(RpcPeer { inner })
    }

    pub async fn call(&self, method: impl Into<String>, params: Value) -> RpcCallResult {
        self.peer()?.call(method, params).await
    }

    pub async fn notify(&self, method: impl Into<String>, params: Value) -> RpcCallResult<()> {
        self.peer()?.notify(method, params).await
    }
}

/// Cheap cloneable handle to a full-duplex RPC connection.
///
/// I/O tasks are started when the peer is created; there must be an active
/// Tokio runtime. Dropping the last owned handle closes the connection.
/// `RpcContext` deliberately holds only a `Weak` reference.
pub struct RpcPeer {
    inner: Arc<PeerInner>,
}

struct PeerInner {
    owner_count: AtomicUsize,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<RpcCallResult>>>,
    pending_slots: Arc<Semaphore>,
    outgoing: mpsc::Sender<RpcMessage>,
    notifications: broadcast::Sender<RpcNotification>,
    events: broadcast::Sender<RpcPeerEvent>,
    shutdown: watch::Sender<bool>,
    closed: AtomicBool,
}

impl PeerInner {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.pending_slots.close();
            self.pending.lock().expect("RPC pending mutex poisoned").clear();
            self.shutdown.send_replace(true);
        }
    }

    fn complete(&self, id: u64, result: RpcResult) {
        let sender = self.pending.lock().expect("RPC pending mutex poisoned").remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(result.map_err(RpcCallError::Remote));
        }
        // Unknown IDs can be responses to calls cancelled by their caller.
    }

    fn event(&self, event: RpcPeerEvent) {
        let _ = self.events.send(event);
    }
}

/// Removes its request from the pending table when a call times out or is
/// cancelled, and releases its pending-request capacity permit.
struct PendingGuard {
    inner: Arc<PeerInner>,
    id: u64,
    _permit: OwnedSemaphorePermit,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.inner.pending.lock().expect("RPC pending mutex poisoned").remove(&self.id);
    }
}

impl RpcPeer {
    /// Start a peer without registered incoming methods. Incoming requests
    /// receive METHOD_NOT_FOUND; notifications remain observable.
    pub fn from_stream<T>(stream: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::from_stream_with_config(stream, None, RpcPeerConfig::default())
    }

    /// Start a peer without registered incoming methods. Incoming requests
    /// receive METHOD_NOT_FOUND; notifications remain observable.
    pub fn from_stream_with_dispatcher<T>(
        stream: T,
        dispatcher: Arc<dyn RpcDispatcher>,
    ) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::from_stream_with_config(stream, Some(dispatcher), RpcPeerConfig::default())
    }

    /// Start a peer without registered incoming methods. Incoming requests
    /// receive METHOD_NOT_FOUND; notifications remain observable.
    pub fn from_stream_with_config<T>(
        stream: T,
        dispatcher: Option<Arc<dyn RpcDispatcher>>,
        config: RpcPeerConfig,
    ) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::from_stream_with_shared_codec(stream, dispatcher, config)
    }

    /// Avoids recreating the codec per accepted server connection.
    pub(crate) fn from_stream_with_shared_codec<T>(
        stream: T,
        dispatcher: Option<Arc<dyn RpcDispatcher>>,
        config: RpcPeerConfig,
    ) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        config.validate();
        let (outgoing, outgoing_rx) = mpsc::channel(config.outbound_capacity);
        let (notifications, _) = broadcast::channel(config.notification_capacity);
        let (events, _) = broadcast::channel(config.event_capacity);
        let (shutdown, _) = watch::channel(false);

        let inner = Arc::new(PeerInner {
            owner_count: AtomicUsize::new(1),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            pending_slots: Arc::new(Semaphore::new(config.max_pending_requests)),
            outgoing,
            notifications,
            events,
            shutdown,
            closed: AtomicBool::new(false),
        });

        let (reader, writer) = tokio::io::split(stream);
        tokio::spawn(read_loop(
            reader, Arc::clone(&inner), dispatcher, config,
        ));
        tokio::spawn(write_loop(
            writer, Arc::clone(&inner), outgoing_rx, config,
        ));

        Self { inner }
    }

    /// Issue a request. Responses may arrive in any order. Dropping this
    /// future removes the pending entry but cannot undo a sent request.
    pub async fn call(&self, method: impl Into<String>, params: Value) -> RpcCallResult {
        if self.is_closed() {
            return Err(RpcCallError::ConnectionClosed);
        }

        // Bound the pending table as well as the outgoing channel.
        let permit = Arc::clone(&self.inner.pending_slots)
            .acquire_owned().await.map_err(|_| RpcCallError::ConnectionClosed)?;
        if self.is_closed() {
            return Err(RpcCallError::ConnectionClosed);
        }

        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            self.close(); // Refuse to reuse request IDs after wraparound.
            return Err(RpcCallError::ConnectionClosed);
        }
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().expect("RPC pending mutex poisoned").insert(id, tx);
        let _registration = PendingGuard {
            inner: Arc::clone(&self.inner), id, _permit: permit,
        };
        if self.is_closed() {
            return Err(RpcCallError::ConnectionClosed);
        }

        self.inner.outgoing.send(RpcMessage::Request {
            id, method: method.into(), params,
        }).await.map_err(|_| RpcCallError::ConnectionClosed)?;

        rx.await.map_err(|_| RpcCallError::ConnectionClosed)?
    }

    /// A local timeout does not mean the remote handler was cancelled.
    pub async fn call_timeout(
        &self,
        method: impl Into<String>,
        params: Value,
        duration: Duration,
    ) -> RpcCallResult {
        tokio::time::timeout(duration, self.call(method, params))
            .await.map_err(|_| RpcCallError::Timeout)?
    }

    /// Success means accepted into the outgoing queue, not delivered.
    pub async fn notify(&self, method: impl Into<String>, params: Value) -> RpcCallResult<()> {
        if self.is_closed() {
            return Err(RpcCallError::ConnectionClosed);
        }
        self.inner.outgoing.send(RpcMessage::Notification {
            method: method.into(), params,
        }).await.map_err(|_| RpcCallError::ConnectionClosed)
    }

    /// Observe incoming notifications independent of router dispatch.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<RpcNotification> {
        self.inner.notifications.subscribe()
    }

    /// Observe notification failures, overload drops, and connection errors.
    pub fn subscribe_events(&self) -> broadcast::Receiver<RpcPeerEvent> {
        self.inner.events.subscribe()
    }

    /// Returns true if the connection is closed or closing. This is a cheap
    /// check that does not guarantee that a subsequent call will succeed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Close the connection and cancel all pending calls. The peer may still be
    /// used to observe notifications and events until all handles are dropped.
    pub fn close(&self) {
        self.inner.close();
    }

    /// Wait for the connection to close and all handlers to exit. This is
    /// useful for tests that want to ensure that all tasks have completed.
    pub async fn wait_closed(&self) {
        let mut rx = self.inner.shutdown.subscribe();
        loop {
            let closed = *rx.borrow_and_update();
            if closed || rx.changed().await.is_err() {
                break;
            }
        }
    }
}

impl Clone for RpcPeer {
    fn clone(&self) -> Self {
        self.inner.owner_count.fetch_add(1, Ordering::Relaxed);
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl Drop for RpcPeer {
    fn drop(&mut self) {
        if self.inner.owner_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.close();
        }
    }
}

async fn read_loop<R>(
    mut reader: R,
    inner: Arc<PeerInner>,
    dispatcher: Option<Arc<dyn RpcDispatcher>>,
    config: RpcPeerConfig,
) where
    R: AsyncRead + Unpin,
{
    let capacity = Arc::new(Semaphore::new(config.max_in_flight));
    let mut handlers = JoinSet::<()>::new();
    let mut shutdown = inner.shutdown.subscribe();

    let outcome: RpcCallResult<()> = loop {
        // Reap without cancelling a partially consumed read_exact().
        while let Some(done) = handlers.try_join_next() {
            if let Err(error) = done {
                inner.event(RpcPeerEvent::ConnectionError(format!(
                    "RPC handler task failed: {error}"
                )));
            }
        }

        let next = tokio::select! {
            _ = shutdown.changed() => break Ok(()),
            frame = read_message(&mut reader, config.max_frame_bytes) => frame,
        };
        // `read_message` is only cancelled when this reader task exits.
        let message = match next {
            Ok(Some(message)) => message,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        };

        match message {
            RpcMessage::Response { id, result } => inner.complete(id, result),
            RpcMessage::Request { id, method, params } => {
                let permit = match Arc::clone(&capacity).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        // Never wait for a handler permit in the reader: an
                        // active handler may itself be waiting for a callback.
                        let rejected = RpcMessage::Response {
                            id, result: Err(RpcError::resource_exhausted()),
                        };
                        if inner.outgoing.try_send(rejected).is_err() {
                            break Err(RpcCallError::ConnectionClosed);
                        }
                        continue;
                    }
                };
                let dispatcher = dispatcher.clone();
                let context = RpcContext::new(&inner);
                let tx = inner.outgoing.clone();
                handlers.spawn(async move {
                    let _permit = permit;
                    let result = match dispatcher {
                        Some(dispatcher) => {
                            dispatcher.dispatch(&context, &method, &params).await
                        }
                        None => Err(RpcError::method_not_found(&method)),
                    };
                    let _ = tx.send(RpcMessage::Response { id, result }).await;
                });
            }
            RpcMessage::Notification { method, params } => {
                let _ = inner.notifications.send(RpcNotification {
                    method: method.clone(), params: params.clone(),
                });
                let Some(dispatcher) = dispatcher.clone() else { continue };
                let permit = match Arc::clone(&capacity).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        inner.event(RpcPeerEvent::NotificationDropped { method });
                        continue;
                    }
                };
                let context = RpcContext::new(&inner);
                let events = inner.events.clone();
                handlers.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = dispatcher.dispatch(&context, &method, &params).await {
                        let _ = events.send(RpcPeerEvent::NotificationFailed { method, error });
                    }
                });
            }
        }
    };

    if let Err(error) = outcome {
        inner.event(RpcPeerEvent::ConnectionError(error.to_string()));
    }
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    inner.close();
}

async fn write_loop<W>(
    mut writer: W,
    inner: Arc<PeerInner>,
    mut outgoing: mpsc::Receiver<RpcMessage>,
    config: RpcPeerConfig,
) where
    W: AsyncWrite + Unpin,
{
    let mut shutdown = inner.shutdown.subscribe();
    let outcome: RpcCallResult<()> = loop {
        let next = tokio::select! {
            _ = shutdown.changed() => break Ok(()),
            next = outgoing.recv() => next,
        };
        let Some(message) = next else { break Ok(()) };
        let result = tokio::select! {
            _ = shutdown.changed() => break Ok(()),
            result = write_message(&mut writer, message, config.max_frame_bytes) => result,
        };
        if let Err(error) = result {
            break Err(error);
        }
    };

    if let Err(error) = outcome {
        inner.event(RpcPeerEvent::ConnectionError(error.to_string()));
    }
    let _ = writer.shutdown().await;
    inner.close();
}
