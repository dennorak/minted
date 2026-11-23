use std::{collections::HashMap, sync::Arc};

use crate::{
    Value, rpc::{RpcContext, RpcDispatcher, RpcError, RpcResult},
};

/// A method implementation on the receiving peer.
///
/// `context` can initiate reverse calls on the *same connection*.
/// It is intentionally weak; storing it does not retain a connection.
#[async_trait::async_trait]
pub trait RpcHandler<S>: Send + Sync {
    fn name(&self) -> &'static str;

    async fn handle(
        &self,
        context: &RpcContext,
        state: &S,
        params: &Value,
    ) -> RpcResult;
}

pub struct RpcRouter<S> {
    handlers: HashMap<String, Box<dyn RpcHandler<S>>>,
}

impl<S> Default for RpcRouter<S> {
    fn default() -> Self { Self::new() }
}

impl<S> RpcRouter<S> {
    pub fn new() -> Self {
        Self { handlers: HashMap::new() }
    }

    /// Replaces a handler with the same name and returns the old handler.
    pub fn register<H>(&mut self, handler: H) -> Option<Box<dyn RpcHandler<S>>>
    where
        H: RpcHandler<S> + 'static,
    {
        self.handlers.insert(handler.name().to_owned(), Box::new(handler))
    }

    pub fn contains(&self, method: &str) -> bool {
        self.handlers.contains_key(method)
    }
}

impl<S: Sync> RpcRouter<S> {
    pub async fn dispatch(
        &self,
        context: &RpcContext,
        state: &S,
        method: &str,
        params: &Value,
    ) -> RpcResult {
        let handler = self.handlers.get(method)
            .ok_or_else(|| RpcError::method_not_found(method))?;
        handler.handle(context, state, params).await
    }
}

/// Adapts a typed router and its state to the type-erased `RpcDispatcher`
/// consumed by `RpcPeer`. Generated server services can register handlers
/// in a router without altering the peer transport.
pub struct RoutedDispatcher<S> {
    state: Arc<S>,
    router: Arc<RpcRouter<S>>,
}

impl<S> RoutedDispatcher<S> {
    pub fn new(state: S, router: RpcRouter<S>) -> Self {
        Self::from_shared(Arc::new(state), Arc::new(router))
    }

    pub fn from_shared(state: Arc<S>, router: Arc<RpcRouter<S>>) -> Self {
        Self { state, router }
    }
}

#[async_trait::async_trait]
impl<S> RpcDispatcher for RoutedDispatcher<S>
where
    S: Send + Sync + 'static,
{
    async fn dispatch(
        &self,
        context: &RpcContext,
        method: &str,
        params: &Value,
    ) -> RpcResult {
        self.router.dispatch(context, self.state.as_ref(), method, params).await
    }
}