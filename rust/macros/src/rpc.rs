//! Per-function Minted RPC attribute macro.
//!
//! ```ignore
//! #[rpc(server, client, nested_under = "player", name = "get")]
//! pub async fn get_player(state: &GameState, id: u64) -> RpcResult<Player> {
//!     state.get_player(id).await
//! }
//! ```
//!
//! This generates a `GetPlayerRpcHandler`, `register_get_player_rpc`,
//! `get_player_client`, and `get_player_context_client`, alongside the original
//! function. The generated handlers implement the existing `RpcHandler<S>`.
//!
//! No custom `RpcValue` trait or runtime conversion module is required:
//! ordinary types use `minted::to_value_named` / `minted::from_value`, while
//! the literal types `Value`, `Values`, and `()` use direct conversions.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{
    Expr, FnArg, GenericArgument, Ident, ItemFn, Lit, LitStr, Meta, Pat, PathArguments, ReturnType,
    Safety, Token, Type, parse::Parser, punctuated::Punctuated,
};

use crate::minted_path;

#[derive(Default)]

struct Options {
    client: bool,
    server: bool,
    notification: bool,
    name: Option<LitStr>,
    nested_under: Option<LitStr>,
    separator: Option<LitStr>,
    client_name: Option<Ident>,
    aliases: Vec<LitStr>,
}

fn string_literal(expr: &Expr, option: &str) -> syn::Result<LitStr> {
    match expr {
        Expr::Lit(syn::ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.clone()),

        _ => Err(syn::Error::new_spanned(
            expr,
            format!("`{option}` expects a string literal"),
        )),
    }
}

fn set_once(target: &mut Option<LitStr>, value: LitStr, name: &str) -> syn::Result<()> {
    if target.is_some() {
        return Err(syn::Error::new_spanned(
            value,
            format!("duplicate `{name}`"),
        ));
    }

    *target = Some(value);

    Ok(())
}

impl Options {
    fn parse(attrs: TokenStream2) -> syn::Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attrs)?;

        let mut options = Self::default();
        let mut aliases_defined = false;

        for meta in metas {
            match meta {
                Meta::Path(path) if path.is_ident("server") => {
                    if options.server {
                        return Err(syn::Error::new_spanned(path, "duplicate `server`"));
                    }

                    options.server = true;
                }

                Meta::Path(path) if path.is_ident("client") => {
                    if options.client {
                        return Err(syn::Error::new_spanned(path, "duplicate `client`"));
                    }

                    options.client = true;
                }

                Meta::Path(path) if path.is_ident("notification") => {
                    if options.notification {
                        return Err(syn::Error::new_spanned(path, "duplicate `notification`"));
                    }

                    options.notification = true;
                }

                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    let name = string_literal(&nv.value, "name")?;

                    set_once(&mut options.name, name, "name")?;
                }

                Meta::NameValue(nv)
                    if nv.path.is_ident("nested_under") || nv.path.is_ident("namespace") =>
                {
                    let namespace = string_literal(&nv.value, "nested_under")?;

                    set_once(
                        &mut options.nested_under,
                        namespace,
                        "nested_under/namespace",
                    )?;
                }

                Meta::NameValue(nv) if nv.path.is_ident("namespace_separator") => {
                    let separator = string_literal(&nv.value, "namespace_separator")?;

                    set_once(&mut options.separator, separator, "namespace_separator")?;
                }

                Meta::NameValue(nv) if nv.path.is_ident("client_name") => {
                    if options.client_name.is_some() {
                        return Err(syn::Error::new_spanned(nv, "duplicate `client_name`"));
                    }

                    let name = string_literal(&nv.value, "client_name")?;

                    options.client_name = Some(
                        syn::parse_str(&name.value())
                            .map_err(|error| syn::Error::new_spanned(&name, error))?,
                    );
                }

                Meta::List(list) if list.path.is_ident("aliases") => {
                    if aliases_defined {
                        return Err(syn::Error::new_spanned(list, "duplicate `aliases`"));
                    }

                    aliases_defined = true;

                    options.aliases = Punctuated::<LitStr, Token![,]>::parse_terminated
                        .parse2(list.tokens)?
                        .into_iter()
                        .collect();
                }

                Meta::NameValue(nv) if nv.path.is_ident("aliases") => {
                    if aliases_defined {
                        return Err(syn::Error::new_spanned(nv, "duplicate `aliases`"));
                    }

                    aliases_defined = true;

                    let Expr::Array(array) = &nv.value else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "`aliases` expects a string array or aliases(\"...\")",
                        ));
                    };

                    options.aliases = array
                        .elems
                        .iter()
                        .map(|expr| string_literal(expr, "aliases"))
                        .collect::<syn::Result<_>>()?;
                }

                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unsupported RPC attribute (expected `server`, `client`, `notification`, `name`, `nested_under`, `namespace`, `namespace_separator`, `client_name`, or `aliases`)",
                    ));
                }
            }
        }

        if !options.server && !options.client {
            options.server = true;

            options.client = true;
        }

        if !options.client && options.client_name.is_some() {
            return Err(syn::Error::new_spanned(
                options.client_name.as_ref().unwrap(),
                "`client_name` requires `client`",
            ));
        }

        if !options.server && !options.aliases.is_empty() {
            return Err(syn::Error::new_spanned(
                &options.aliases[0],
                "`aliases` requires `server`",
            ));
        }

        Ok(options)
    }

    fn wire_names(&self, function: &Ident) -> syn::Result<(LitStr, Vec<LitStr>)> {
        let separator = self
            .separator
            .as_ref()
            .map(LitStr::value)
            .unwrap_or_else(|| ".".into());

        if separator.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`namespace_separator` cannot be empty",
            ));
        }

        let leaf = self
            .name
            .as_ref()
            .map(LitStr::value)
            .unwrap_or_else(|| function.to_string().trim_start_matches("r#").to_owned());

        if leaf.is_empty() {
            return Err(syn::Error::new_spanned(
                function,
                "RPC method name cannot be empty",
            ));
        }

        let namespace = self
            .nested_under
            .as_ref()
            .map(LitStr::value)
            .unwrap_or_default();

        let namespace = namespace
            .trim_start_matches(separator.as_str())
            .trim_end_matches(separator.as_str());

        let qualify = |name: &str| {
            if namespace.is_empty() {
                name.to_owned()
            } else {
                format!("{namespace}{separator}{name}")
            }
        };

        let name = LitStr::new(&qualify(&leaf), Span::call_site());

        let mut names = HashSet::from([name.value()]);

        let mut aliases = Vec::new();

        for alias in &self.aliases {
            if alias.value().is_empty() {
                return Err(syn::Error::new_spanned(alias, "RPC alias cannot be empty"));
            }

            let full = qualify(&alias.value());

            if !names.insert(full.clone()) {
                return Err(syn::Error::new_spanned(
                    alias,
                    "duplicate RPC name or alias",
                ));
            }

            aliases.push(LitStr::new(&full, alias.span()));
        }

        Ok((name, aliases))
    }
}

#[derive(Clone)]
struct Parameter {
    name: Ident,
    ty: Type,
}

#[derive(Clone, Copy)]
enum Injected {
    Context,
    State,
}

struct Analyzed {
    injected: Vec<Injected>,
    state_type: Option<Type>,
    params: Vec<Parameter>,
    result_type: Type,
}

fn type_is(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|seg| seg.ident == name))
}

fn shared_ref(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(reference) if reference.mutability.is_none() => Some(&reference.elem),

        _ => None,
    }
}

fn require_ident(pat: &Pat) -> syn::Result<Ident> {
    match pat {
        Pat::Ident(ident) if ident.by_ref.is_none() && ident.subpat.is_none() => {
            Ok(ident.ident.clone())
        }

        _ => Err(syn::Error::new_spanned(
            pat,
            "RPC parameters must be simple named arguments",
        )),
    }
}

fn result_type(return_type: &ReturnType) -> syn::Result<Type> {
    let ReturnType::Type(_, full_type) = return_type else {
        return Err(syn::Error::new_spanned(
            return_type,
            "RPC functions must return `RpcResult<T>`",
        ));
    };

    let Type::Path(path) = full_type.as_ref() else {
        return Err(syn::Error::new_spanned(
            full_type,
            "expected `RpcResult<T>`",
        ));
    };

    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            full_type,
            "expected `RpcResult<T>`",
        ));
    };

    if segment.ident != "RpcResult" {
        return Err(syn::Error::new_spanned(
            full_type,
            "expected `RpcResult<T>`",
        ));
    }

    match &segment.arguments {
        PathArguments::None => {
            let minted = minted_path();

            Ok(syn::parse_quote!(#minted::Value))
        }

        PathArguments::AngleBracketed(arguments) if arguments.args.len() == 1 => {
            if let GenericArgument::Type(ty) = arguments.args.first().unwrap() {
                Ok(ty.clone())
            } else {
                Err(syn::Error::new_spanned(
                    arguments,
                    "expected one result type",
                ))
            }
        }

        _ => Err(syn::Error::new_spanned(
            full_type,
            "expected `RpcResult<T>`",
        )),
    }
}

fn analyze(function: &ItemFn, options: &Options) -> syn::Result<Analyzed> {
    let sig = &function.sig;

    if sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            sig,
            "`#[rpc]` requires an async function",
        ));
    }

    if sig.constness.is_some()
        || matches!(sig.safety, Safety::Unsafe(..))
        || sig.abi.is_some()
        || sig.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            sig,
            "unsupported RPC function qualifier",
        ));
    }

    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "generic RPC functions are unsupported",
        ));
    }

    let result_type = result_type(&sig.output)?;

    if options.notification && !matches!(&result_type, Type::Tuple(tuple) if tuple.elems.is_empty())
    {
        return Err(syn::Error::new_spanned(
            &sig.output,
            "RPC notifications must return `RpcResult<()>`",
        ));
    }

    let mut injected = Vec::new();
    let mut state_type = None;
    let mut params = Vec::new();
    let mut seen_wire = false;
    let mut seen_context = false;

    for input in &sig.inputs {
        let FnArg::Typed(typed) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "`#[rpc]` works on free functions, not methods",
            ));
        };

        let name = require_ident(&typed.pat)?;

        let ty = (*typed.ty).clone();

        if name == "ctx" || name == "context" {
            if seen_wire || seen_context {
                return Err(syn::Error::new_spanned(
                    name,
                    "a context parameter must appear once, before wire parameters",
                ));
            }

            if !shared_ref(&ty).is_some_and(|ty| type_is(ty, "RpcContext")) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "`ctx`/`context` must have type `&RpcContext`",
                ));
            }

            injected.push(Injected::Context);

            seen_context = true;
        } else if name == "state" {
            if seen_wire || state_type.is_some() {
                return Err(syn::Error::new_spanned(
                    name,
                    "a state parameter must appear once, before wire parameters",
                ));
            }

            let Some(inner) = shared_ref(&ty) else {
                return Err(syn::Error::new_spanned(ty, "`state` must be `&StateType`"));
            };

            state_type = Some(inner.clone());

            injected.push(Injected::State);
        } else {
            seen_wire = true;

            if matches!(ty, Type::Reference(_)) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "wire parameters must be owned types, not references",
                ));
            }

            if name.to_string().starts_with("__rpc_") {
                return Err(syn::Error::new_spanned(
                    name,
                    "parameter names starting with `__rpc_` are reserved",
                ));
            }

            params.push(Parameter { name, ty });
        }
    }

    Ok(Analyzed {
        injected,
        state_type,
        params,
        result_type,
    })
}

/// Rust syntax recognition, not semantic type resolution. Aliases of `Value`
/// and nested types like `Vec<Value>` are treated as normal Serde wire types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conversion {
    Value,
    Values,
    Unit,
    Serde,
}

fn conversion(ty: &Type) -> Conversion {
    if matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty()) {
        return Conversion::Unit;
    }

    let Type::Path(path) = ty else {
        return Conversion::Serde;
    };

    if path.qself.is_some() {
        return Conversion::Serde;
    }

    let segments: Vec<_> = path.path.segments.iter().collect();

    let last = match segments.as_slice() {
        [last] => last,

        [prefix, last] if prefix.ident == "minted" || prefix.ident == "crate" => last,

        _ => return Conversion::Serde,
    };

    if !matches!(last.arguments, PathArguments::None) {
        return Conversion::Serde;
    }

    if last.ident == "Value" {
        Conversion::Value
    } else if last.ident == "Values" {
        Conversion::Values
    } else {
        Conversion::Serde
    }
}

fn title_case(ident: &Ident) -> String {
    let mut result = String::new();

    for segment in ident.to_string().trim_start_matches("r#").split('_') {
        let mut chars = segment.chars();

        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());

            result.extend(chars);
        }
    }

    result
}

/// Entry point for the `#[proc_macro_attribute] rpc` function in lib.rs.

pub fn derive(attrs: TokenStream, item: TokenStream) -> syn::Result<TokenStream2> {
    let options = Options::parse(attrs.into())?;

    let function = syn::parse::<ItemFn>(item)?;

    expand(function, options)
}

fn expand(function: ItemFn, options: Options) -> syn::Result<TokenStream2> {
    let info = analyze(&function, &options)?;
    let minted = minted_path();
    let original_name = &function.sig.ident;
    let vis = &function.vis;
    let (method_name, aliases) = options.wire_names(original_name)?;
    let handler_name = format_ident!("{}RpcHandler", title_case(original_name));
    let register_name = format_ident!("register_{}_rpc", original_name);
    let client_name = options
        .client_name
        .clone()
        .unwrap_or_else(|| format_ident!("{}_client", original_name));
    let context_client_name = format_ident!("{}_context_client", original_name);
    let state = info.state_type.as_ref();
    let returned = &info.result_type;
    let count = info.params.len();
    let names: Vec<_> = info.params.iter().map(|param| &param.name).collect();
    let types: Vec<_> = info.params.iter().map(|param| &param.ty).collect();

    let args: Vec<_> = info
        .injected
        .iter()
        .map(|item| match item {
            Injected::Context => quote!(__rpc_context),

            Injected::State => quote!(__rpc_state),
        })
        .chain(names.iter().map(|name| quote!(#name)))
        .collect();

    let cfg_attrs: Vec<_> = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .collect();

    let decode_args: Vec<_> = names
        .iter()
        .zip(types.iter())
        .enumerate()
        .map(|(index, (name, ty))| {
            let value = quote!(__rpc_args[#index].clone());

            let decode = match conversion(ty) {
                Conversion::Value => value,
                Conversion::Values => quote! {
                    match #value {
                        #minted::Value::Object(fields) => fields,
                        _ => return Err(#minted::rpc::RpcError::invalid_params(
                            concat!("expected object for argument `", stringify!(#name), "`")
                        )),
                    }
                },

                Conversion::Unit => quote! {
                    match #value {
                        #minted::Value::Null => (),
                        _ => return Err(#minted::rpc::RpcError::invalid_params(
                            concat!("expected null for argument `", stringify!(#name), "`")
                        )),
                    }
                },

                Conversion::Serde => quote! {
                    #minted::from_value(#value).map_err(|error| {
                        #minted::rpc::RpcError::invalid_params(format!(
                            "invalid argument `{}`: {}", stringify!(#name), error
                        ))
                    })?
                },
            };

            quote! { let #name: #ty = #decode; }
        })
        .collect();

    let encode_args: Vec<_> = names
        .iter()
        .zip(types.iter())
        .map(|(name, ty)| match conversion(ty) {
            Conversion::Value => quote!(#name),
            Conversion::Values => quote!(#minted::Value::Object(#name)),
            Conversion::Unit => quote!(#minted::Value::Null),
            Conversion::Serde => quote! {
                #minted::to_value_named(&#name).map_err(|error| {
                    #minted::rpc::RpcCallError::Protocol(
                        #minted::rpc::RpcProtocolError::Encode(error.to_string())
                    )
                })?
            },
        })
        .collect();

    let encode_result = match conversion(returned) {
        Conversion::Value => quote!(__rpc_result),
        Conversion::Values => quote!(#minted::Value::Object(__rpc_result)),
        Conversion::Unit => quote!(#minted::Value::Null),
        Conversion::Serde => quote! {
                    #minted::to_value_named(&__rpc_result).map_err(|_error| {
                        // Do not leak internal serialization state to remote callers.
                        #minted::rpc::RpcError::internal()
                    })?
                },
    };

    let decode_result = match conversion(returned) {
        Conversion::Value => quote!(Ok(__rpc_reply)),
        Conversion::Values => quote! {
            match __rpc_reply {
                #minted::Value::Object(fields) => Ok(fields),
                _ => Err(#minted::rpc::RpcCallError::Protocol(
                    #minted::rpc::RpcProtocolError::Decode(
                        format!("expected object in response to `{}`", #method_name)
                    )
                )),
            }
        },

        Conversion::Unit => quote! {
            match __rpc_reply {
                #minted::Value::Null => Ok(()),
                _ => Err(#minted::rpc::RpcCallError::Protocol(
                    #minted::rpc::RpcProtocolError::Decode(
                        format!("expected null in response to `{}`", #method_name)
                    )
                )),
            }
        },

        Conversion::Serde => quote! {
            #minted::from_value(__rpc_reply).map_err(|error| {
                #minted::rpc::RpcCallError::Protocol(
                    #minted::rpc::RpcProtocolError::Decode(format!(
                        "invalid reply from `{}`: {}", #method_name, error
                    ))
                )
            })
        },
    };

    // Stateless functions can be registered in any RpcRouter<S>.

    let (impl_generics, handler_state, register_generics) = if let Some(state) = state {
        (quote!(), quote!(#state), quote!())
    } else {
        (
            quote!(<__RpcState: Send + Sync + 'static>),
            quote!(__RpcState),
            quote!(<__RpcState: Send + Sync + 'static>),
        )
    };

    let server = if options.server {
        quote! {
            #(#cfg_attrs)*
            #[doc = "Generated server handler for this RPC function."]
            #vis struct #handler_name {
                name: &'static str,
            }

            #(#cfg_attrs)*
            impl ::core::default::Default for #handler_name {
                fn default() -> Self {
                    Self { name: #method_name }
                }
            }

            #(#cfg_attrs)*
            #[#minted::rpc::async_trait]
            impl #impl_generics #minted::rpc::RpcHandler<#handler_state> for #handler_name {
                fn name(&self) -> &'static str { self.name }

                async fn handle(
                    &self,
                    __rpc_context: &#minted::rpc::RpcContext,
                    __rpc_state: &#handler_state,
                    __rpc_params: &#minted::Value,

                ) -> #minted::rpc::RpcResult {
                    let #minted::Value::List(__rpc_args) = __rpc_params else {
                        return Err(#minted::rpc::RpcError::invalid_params(
                            "expected positional parameter list"
                        ));
                    };

                    if __rpc_args.len() != #count {
                        return Err(#minted::rpc::RpcError::invalid_params(format!(
                            "expected {} arguments, received {}", #count, __rpc_args.len()
                        )));
                    }

                    #(#decode_args)*

                    let __rpc_result = #original_name(#(#args),*).await?;
                    Ok(#encode_result)
                }
            }

            #(#cfg_attrs)*
            #[doc = "Registers the RPC method and its aliases."]
            #vis fn #register_name #register_generics(
                __rpc_router: &mut #minted::rpc::RpcRouter<#handler_state>,
            ) {
                __rpc_router.register(#handler_name::default());
                #(
                    __rpc_router.register(#handler_name { name: #aliases });
                )*
            }
        }
    } else {
        quote!()
    };

    let client = if options.client {
        let invoke = if options.notification {
            quote! {
                __rpc_peer.notify(#method_name, __rpc_params).await
            }
        } else {
            quote! {
                let __rpc_reply = __rpc_peer.call(#method_name, __rpc_params).await?;
                #decode_result
            }
        };

        let return_type = if options.notification {
            quote!(#minted::rpc::RpcCallResult<()>)
        } else {
            quote!(#minted::rpc::RpcCallResult<#returned>)
        };

        quote! {
            #(#cfg_attrs)*
            #[doc = "Typed RPC call over a bidirectional `RpcPeer`."]
            #vis async fn #client_name(
                __rpc_peer: &#minted::rpc::RpcPeer,
                #(#names: #types),*
            ) -> #return_type {
                let __rpc_params = #minted::Value::List(::std::vec![#(#encode_args),*]);
                #invoke
            }

            #(#cfg_attrs)*
            #[doc = "Typed reverse RPC call from an incoming handler's `RpcContext`."]
            #vis async fn #context_client_name(
                __rpc_context: &#minted::rpc::RpcContext,
                #(#names: #types),*
            ) -> #return_type {
                let __rpc_peer = __rpc_context.peer()?;
                #client_name(&__rpc_peer, #(#names),*).await
            }
        }
    } else {
        quote!()
    };

    // A companion *module* can share a name with a free function in Rust:
    // modules live in the type namespace; functions live in the value namespace.
    // This keeps generated names discoverable without introducing a service
    // struct for every single RPC method.
    let register_api = if options.server {
        quote! {
            /// Register this method (including its aliases) on a router.
            pub fn register #register_generics(
                router: &mut #minted::rpc::RpcRouter<#handler_state>,
            ) {
                super::#register_name(router);
            }
        }
    } else {
        quote!()
    };

    let client_api = if options.client {
        let client_return = if options.notification {
            quote!(#minted::rpc::RpcCallResult<()>)
        } else {
            quote!(#minted::rpc::RpcCallResult<#returned>)
        };

        quote! {
            /// Typed calls for this RPC method on either kind of connection handle.
            /// Import with `use path::to::method::Client as _;`.
            #[allow(async_fn_in_trait)]
            pub trait Client {
                async fn #original_name(&self, #(#names: #types),*) -> #client_return;
            }

            impl Client for #minted::rpc::RpcPeer {
                async fn #original_name(&self, #(#names: #types),*) -> #client_return {
                    super::#client_name(self, #(#names),*).await
                }
            }

            impl Client for #minted::rpc::RpcContext {
                async fn #original_name(&self, #(#names: #types),*) -> #client_return {
                    super::#context_client_name(self, #(#names),*).await
                }
            }
        }
    } else {
        quote!()
    };

    // `cfg` also needs to guard this module, not just the original and adapters.
    let api = quote! {
        #(#cfg_attrs)*
        #vis mod #original_name {
            #[allow(unused_imports)]
            use super::*;

            #register_api
            #client_api
        }
        pub use #original_name::Client as _;
    };

    Ok(quote! { #function #server #client #api })
}

#[cfg(test)]

mod tests {

    use super::*;

    fn expand_source(attributes: TokenStream2, source: TokenStream2) -> syn::Result<TokenStream2> {
        let options = Options::parse(attributes)?;

        expand(syn::parse2::<ItemFn>(source)?, options)
    }

    #[test]

    fn nested_names_aliases_and_separator() {
        let options = Options::parse(quote!(
            server,
            client,
            nested_under = "game.player.",
            name = "get",
            aliases("fetch", "lookup")
        ))
        .unwrap();

        let (name, aliases) = options.wire_names(&format_ident!("get_player")).unwrap();

        assert_eq!(name.value(), "game.player.get");
        assert_eq!(aliases[0].value(), "game.player.fetch");
        assert_eq!(aliases[1].value(), "game.player.lookup");

        let options = Options::parse(quote!(
            server,
            nested_under = "game::player::",
            namespace_separator = "::"
        ))
        .unwrap();

        assert_eq!(
            options
                .wire_names(&format_ident!("ping"))
                .unwrap()
                .0
                .value(),
            "game::player::ping"
        );
    }

    #[test]

    fn server_expansion_targets_real_router_signature() {
        let tokens = expand_source(
            quote!(
                server,
                nested_under = "player",
                name = "get",
                aliases("fetch")
            ),
            quote! {

                pub async fn get_player(

                    ctx: &RpcContext, state: &GameState, id: u64, raw: Value

                ) -> RpcResult<Value> {

                    let _ = (ctx, state, id);

                    Ok(raw)

                }

            },
        )
        .unwrap();

        let generated = tokens.to_string();

        assert!(generated.contains("register_get_player_rpc"));
        assert!(generated.contains("RpcHandler"));
        assert!(generated.contains("player.get"));
        assert!(generated.contains("player.fetch"));
        assert!(generated.contains("__rpc_context"));
        assert!(generated.contains("__rpc_state"));
        assert!(!generated.contains("get_player_client"));

        syn::parse2::<syn::File>(tokens).unwrap();
    }

    #[test]

    fn both_sides_use_minted_conversions_not_custom_trait() {
        let tokens = expand_source(
            quote!(server, client, nested_under = "world"),
            quote! {

                async fn get_info(id: u64, input: Value) -> RpcResult<MyType> {

                    let _ = (id, input);

                    todo!()

                }

            },
        )
        .unwrap();

        let generated = tokens.to_string();

        assert!(generated.contains("to_value_named"));
        assert!(generated.contains("from_value"));
        assert!(!generated.contains("RpcValue"));
        assert!(generated.contains("get_info_client"));
        assert!(generated.contains("get_info_context_client"));

        syn::parse2::<syn::File>(tokens).unwrap();
    }

    #[test]

    fn notifications_only_send_one_way_messages() {
        let tokens = expand_source(
            quote!(client, notification),
            quote!(
                async fn ping(id: u64) -> RpcResult<()> {
                    let _ = id;
                    Ok(())
                }
            ),
        )
        .unwrap();

        assert!(tokens.to_string().contains("notify"));
        assert!(!tokens.to_string().contains("register_ping_rpc"));

        syn::parse2::<syn::File>(tokens).unwrap();
    }

    #[test]
    fn companion_module_exposes_client_trait_and_registration() {
        let tokens = expand_source(
            quote!(),
            quote!(
                pub async fn ping(value: String) -> RpcResult<String> {
                    Ok(value)
                }
            ),
        )
        .unwrap();
        let source = tokens.to_string();
        assert!(source.contains("mod ping"));
        assert!(source.contains("trait Client"));
        assert!(source.contains("super :: register_ping_rpc"));
        assert!(source.contains("super :: ping_client"));
        assert!(source.contains("super :: ping_context_client"));
        syn::parse2::<syn::File>(tokens).unwrap();
    }

    #[test]

    fn bad_signatures_are_rejected() {
        assert!(
            expand_source(
                quote!(server),
                quote!(
                    fn ping() -> RpcResult<()> {
                        Ok(())
                    }
                ),
            )
            .is_err()
        );

        assert!(
            expand_source(
                quote!(server),
                quote!(
                    async fn ping(foo: &str) -> RpcResult<()> {
                        Ok(())
                    }
                ),
            )
            .is_err()
        );

        assert!(
            expand_source(
                quote!(notification),
                quote!(
                    async fn ping() -> RpcResult<u64> {
                        Ok(0)
                    }
                ),
            )
            .is_err()
        );
    }
}
