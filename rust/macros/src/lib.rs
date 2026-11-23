#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::panic, clippy::disallowed_methods)
)]
#![warn(
    missing_docs,
    clippy::all,
    clippy::nursery,
    clippy::todo,
    clippy::print_literal,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro,
    clippy::unimplemented
)]
#![cfg_attr(not(doctest), doc = include_str!("../../../readme.md"))]
#![cfg_attr(not(debug_assertions), deny(warnings))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use proc_macro::TokenStream;
#[cfg(feature = "introspect")]
use proc_macro_crate::{FoundCrate, crate_name};
#[cfg(feature = "introspect")]
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, format_ident};
use syn::{DeriveInput, Ident, Type, parse_macro_input};

mod value;
#[cfg(feature = "introspect")]
mod introspect;

#[cfg(feature = "introspect")]
fn minted_path() -> TokenStream2
{
	match crate_name("minted").expect("couldn't find minted crate")
	{
		FoundCrate::Itself => quote::quote!(minted),
		FoundCrate::Name(name) =>
		{
			let ident = Ident::new(&name, Span::call_site());
			quote::quote!(::#ident)
		}
	}
}

fn default_static_ident(ty: &Type) -> Ident {
    let s = ty
        .to_token_stream()
        .to_string()
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || c == '_' => c,
            _ => '_',
        })
        .collect::<String>();

    format_ident!("TAG_DEFAULT_{}", s)
}

#[proc_macro_derive(MintValue, attributes(mint, default))]
/// Derives the `MintValue` trait for an enum.
pub fn derive_mint_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match value::derive(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[cfg(feature = "introspect")]
#[proc_macro_derive(Introspect, attributes(mint))]
/// Derives the `Introspect` trait for a struct or enum, enabling the generation of a schema representation for the type's associated value object.
pub fn derive_mint_introspect(input: TokenStream) -> TokenStream
{
	let input = parse_macro_input!(input as DeriveInput);

	match introspect::derive(&input)
	{
		Ok(tokens) => tokens.into(),
		Err(error) => error.into_compile_error().into(),
	}
}