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
use syn::{DeriveInput, parse_macro_input};

mod value;

#[proc_macro_derive(MintValue, attributes(mint, default))]
/// Derives the `MintValue` trait for an enum.
pub fn derive_mint_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match value::derive(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
