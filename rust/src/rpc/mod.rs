//! Transport-independent, bidirectional async RPC for Minted.
//!
//! `RpcPeer` is the connection runtime. `RpcRouter<S>` is the local method
//! registry; `RoutedDispatcher<S>` erases its application-state type. The
//! client and server modules only initiate/accept connections.

use crate::Value;

mod codec;
mod error;
mod peer;
mod router;
pub use router::*;

pub use async_trait::async_trait;
pub use minted_macros::rpc;
#[cfg(feature = "rpc-tcp")]
pub mod tcp;

pub use error::{RpcCallError, RpcCallResult, RpcError, RpcErrorCode, RpcProtocolError, RpcResult};
pub use peer::{RpcContext, RpcDispatcher, RpcNotification, RpcPeer, RpcPeerConfig, RpcPeerEvent};

/// RPC envelopes are unchanged by bidirectionality. Both peers may send
/// requests; response IDs are scoped to the sender's connection state.
/// A single RPC message. Request IDs are scoped to a connection.
#[derive(Debug, PartialEq, Clone)]
pub enum RpcMessage
{
	/// A method invocation that expects a response.
	Request
	{
		/// The ID of the request.
		id:     u64,
		/// The name of the method to invoke.
		method: String,
		/// The parameters to pass to the method.
		params: Value,
	},
	/// A completed request, including application-level failures.
	Response
	{
		/// The ID of the request being responded to.
		id:     u64,
		/// The result of the request, if it succeeded.
		///
		/// This is serialized as two fields: `result` for success and `error` for failure.
		/// Only one of these fields will be present in the serialized message.
		result: RpcResult<Value>,
	},
	/// A one-way method invocation.
	Notification
	{
		/// The name of the method to invoke.
		method: String,
		/// The parameters to pass to the method.
		params: Value,
	},
}

#[cfg(test)]
mod tests;
