use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Error, Lifetime, LifetimeParam, Result, Type};

use crate::{
	introspect::{EnumVariant, Field, SchemaInput},
	minted_path,
};

pub enum ViewInput<'a>
{
	Struct
	{
		fields: &'a [Field]
	},
	Enum
	{
		is_tupl: bool,
		variants: &'a [EnumVariant]
	},
}

pub fn generate_views(input: &SchemaInput, lifetime: &Lifetime, view_input: ViewInput) -> Result<TokenStream2>
{
	let name = &input.ident;
	let vis = &input.vis;
	let generics = &input.generics;
	let view_name = format_ident!("{name}View");
	let view_mut_name = format_ident!("{name}ViewMut");

	let mut view_generics = generics.clone();
	view_generics
		.params
		.insert(0, syn::GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())));
	let (_, _, view_where) = view_generics.split_for_impl();

	let (kw, mutable, immutable) = match view_input
	{
		ViewInput::Struct { fields } =>
		{
			let immutable = fields
				.iter()
				.map(|field| {
					let ident = &field.ident;
					let vis = &field.vis;
					let ty = field_view_type(field, lifetime, false)?;

					Ok::<_, Error>(quote! {
						#vis #ident: #ty
					})
				})
				.collect::<Result<Vec<_>>>()?;

			let mutable = fields
				.iter()
				.map(|field| {
					let ident = &field.ident;
					let vis = &field.vis;
					let ty = field_view_type(field, lifetime, true)?;

					Ok::<_, Error>(quote! {
						#vis #ident: #ty
					})
				})
				.collect::<Result<Vec<_>>>()?;

			(quote! { struct }, mutable, immutable)
		}
		ViewInput::Enum { variants, is_tupl } =>
		{
			let immutable_variants = generate_enum_view_variants(variants, &lifetime, false, is_tupl)?;

			let mutable_variants = generate_enum_view_variants(variants, &lifetime, true, is_tupl)?;

			(quote! { enum }, mutable_variants, immutable_variants)
		}
	};


	Ok(quote! {
		#vis #kw #view_name #view_generics #view_where {
			#(#immutable,)*
		}

		#vis #kw #view_mut_name #view_generics #view_where {
			#(#mutable,)*
		}
	})
}

fn field_view_type(field: &Field, lifetime: &Lifetime, mutable: bool) -> Result<Type>
{
	let ty = &field.ty;
	let minted = minted_path();

	if field.optional
	{
		let view = if mutable
		{
			quote!(<#ty as #minted::introspect::IntrospectValue<#lifetime>>::ViewMut)
		}
		else
		{
			quote!(<#ty as #minted::introspect::IntrospectValue<#lifetime>>::View)
		};

		Ok(syn::parse2(quote!(Option<#view>))?)
	}
	else
	{
		let view = if mutable
		{
			quote!(<#ty as #minted::introspect::IntrospectValue<#lifetime>>::ViewMut)
		}
		else
		{
			quote!(<#ty as #minted::introspect::IntrospectValue<#lifetime>>::View)
		};

		Ok(syn::parse2(view)?)
	}
}

fn generate_enum_view_variants(
	variants: &[EnumVariant],
	lifetime: &Lifetime,
	mutable: bool,
	is_tupl: bool,
) -> Result<Vec<TokenStream2>>
{
	variants
		.iter()
		.map(|variant| {
			let ident = &variant.ident;

			let fields = variant
				.fields
				.iter()
				.map(|field| {
					let field_ident = &field.ident;
					let vis = &field.vis;

					let ty = field_view_type(field, lifetime, mutable)?;

					if is_tupl {
						Ok::<_, Error>(quote! {
							#vis #ty
						})
					} else {
						Ok::<_, Error>(quote! {
							#vis #field_ident: #ty
						})
					}
				})
				.collect::<Result<Vec<_>>>()?;

			if is_tupl {
				Ok(quote! {
					#ident (
						#(#fields),*
					)
				})
			} else {
				Ok(quote! {
					#ident {
						#(#fields,)*
					}
				})
			}
		})
		.collect()
}
