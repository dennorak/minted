#![allow(clippy::option_if_let_else, reason = "darling (lol)")]

use darling::{FromDeriveInput, FromVariant, ast};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{Attribute, DeriveInput, Error, Ident, Result, Type};

use crate::default_static_ident;

#[derive(Debug, FromDeriveInput)]
#[darling(supports(enum_any))]
struct MintValueInput
{
	ident: Ident,
	vis:   syn::Visibility,

	data: ast::Data<TagVariant, ()>,
}

#[derive(Debug, FromVariant)]
#[darling(attributes(mint), forward_attrs(cfg, doc, default))]
struct TagVariant
{
	ident:  Ident,
	fields: ast::Fields<syn::Type>,

	id: syn::LitInt,

	#[cfg(feature = "introspect")]
	#[darling(default)]
	exclude_schema: bool,

	#[darling(default)]
	exclude_from: bool,
	#[darling(default)]
	extra_from:   Vec<Ident>,

	attrs: Vec<Attribute>,
}

impl TagVariant
{
	fn cfg_attributes(&self) -> impl Iterator<Item = &Attribute>
	{
		self.attrs.iter().filter(|attr| attr.path().is_ident("cfg"))
	}

	fn doc_attributes(&self) -> impl Iterator<Item = &Attribute>
	{
		self.attrs.iter().filter(|attr| attr.path().is_ident("doc"))
	}

	fn default_attributes(&self) -> impl Iterator<Item = &Attribute>
	{
		self.attrs.iter().filter(|attr| attr.path().is_ident("default"))
	}

	fn single_field(&self) -> Result<Option<&Type>>
	{
		match &self.fields.style
		{
			ast::Style::Unit => Ok(None),

			ast::Style::Tuple if self.fields.fields.len() == 1 => Ok(Some(&self.fields.fields[0])),

			ast::Style::Tuple => Err(Error::new_spanned(
				&self.fields,
				"MintValue only supports unit variants or variants with one field",
			)),

			ast::Style::Struct => Err(Error::new_spanned(
				&self.fields,
				"MintValue does not support struct variants",
			)),
		}
	}
}

pub fn derive(input: &DeriveInput) -> Result<TokenStream2>
{
	let parsed = MintValueInput::from_derive_input(input)?;

	let enum_name = &parsed.ident;
	let enum_kind_name = format_ident!("{}Kind", enum_name);
	let viz = &parsed.vis;

	let variants = match &parsed.data
	{
		ast::Data::Enum(variants) => variants,
		_ =>
		{
			return Err(Error::new_spanned(input, "MintValue can only be derived for enums"));
		}
	};

	let mut segments = Vec::new();
	let mut ids = Vec::new();
	let mut id_branches = Vec::new();
	let mut to_kind_branches = Vec::new();
	let mut from_kind_branches = Vec::new();
	let mut kind_name_branches = Vec::new();
	let mut kind_variants = Vec::new();
	let mut all_kinds = Vec::new();
	let mut pushed_defaults = Vec::new();

	for variant in variants
	{
		let name = &variant.ident;
		let id = &variant.id;

		let cfg_attributes: Vec<_> = variant.cfg_attributes().collect();
		let doc_attributes: Vec<_> = variant.doc_attributes().collect();
		let default_attributes: Vec<_> = variant.default_attributes().collect();

		match variant.single_field()?
		{
			None =>
			{
				// Unit variant
				segments.push(quote! {
					#(#cfg_attributes)*
					impl From<()> for #enum_name {
						fn from(_: ()) -> Self {
							Self::#name
						}
					}

					#(#cfg_attributes)*
					impl TryFrom<#enum_name> for () {
						type Error = ();

						fn try_from(value: #enum_name) -> Result<Self, Self::Error> {
							match value {
								#enum_name::#name => Ok(()),
								_ => Err(()),
							}
						}
					}

					#(#cfg_attributes)*
					impl<'a> TryFrom<&'a #enum_name> for &'a () {
						type Error = ();

						fn try_from(value: &'a #enum_name) -> Result<Self, Self::Error> {
							match value {
								#enum_name::#name => Ok(&()),
								_ => Err(()),
							}
						}
					}
				});

				id_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_name::#name => #id,
				});

				ids.push(id);

				to_kind_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_name::#name => #enum_kind_name::#name,
				});

				from_kind_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name => #enum_name::#name,
				});

				kind_name_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name => stringify!(#name),
				});

				kind_variants.push(quote! {
					#(#default_attributes)*
					#(#cfg_attributes)*
					#(#doc_attributes)*
					#name,
				});

				all_kinds.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name,
				});
			}

			Some(ty) =>
			{
				// Single-field tuple variant
				let value = Ident::new("value", Span::call_site());
				let static_default_name = default_static_ident(ty);

				if !pushed_defaults.contains(&static_default_name)
				{
					pushed_defaults.push(static_default_name.clone());

					segments.push(quote! {
						/// A static default value for the tag.
						#[allow(non_upper_case_globals)]
						pub static #static_default_name:
							::std::sync::LazyLock<#ty> =
							::std::sync::LazyLock::new(|| {
								::std::default::Default::default()
							});
					});
				}

				if !variant.exclude_from
				{
					segments.push(quote! {
						#(#cfg_attributes)*
						impl From<#ty> for #enum_name {
							fn from(#value: #ty) -> Self {
								Self::#name(#value)
							}
						}

						#(#cfg_attributes)*
						impl TryFrom<#enum_name> for #ty {
							type Error = ();

							fn try_from(value: #enum_name) -> Result<Self, Self::Error> {
								match value {
									#enum_name::#name(value) => Ok(value),
									_ => Err(()),
								}
							}
						}

						#(#cfg_attributes)*
						impl<'a> TryFrom<&'a #enum_name> for &'a #ty {
							type Error = ();

							fn try_from(value: &'a #enum_name) -> Result<Self, Self::Error> {
								match value {
									#enum_name::#name(value) => Ok(value),
									_ => Err(()),
								}
							}
						}

						#(#cfg_attributes)*
						impl<'a> TryFrom<&'a mut #enum_name> for &'a mut #ty {
							type Error = ();

							fn try_from(
								value: &'a mut #enum_name,
							) -> Result<Self, Self::Error> {
								match value {
									#enum_name::#name(value) => Ok(value),
									_ => Err(()),
								}
							}
						}
					});
				}

				#[cfg(feature = "introspect")]
				if !variant.exclude_schema
				{
					segments.push(quote! {
						impl<'a> crate::introspect::IntrospectValue<'a> for #ty {
							type View = &'a Self;
							type ViewMut = &'a mut Self;

							fn view(
								tag: &'a crate::Value,
							) -> Result<Self::View, crate::introspect::ValueViewError> {
								match tag {
									crate::Value::#name(value) => Ok(value),
									_ => Err(crate::introspect::ValueViewError::InvalidCast),
								}
							}

							fn view_mut(
								tag: &'a mut crate::Value,
							) -> Result<Self::ViewMut, crate::introspect::ValueViewError> {
								match tag {
									crate::Value::#name(value) => Ok(value),
									_ => Err(crate::introspect::ValueViewError::InvalidCast),
								}
							}

							fn from_tag(
								tag: &'a crate::Value,
							) -> Result<Self, crate::introspect::ValueViewError> {
								match tag {
									crate::Value::#name(value) => Ok(value.clone()),
									_ => Err(crate::introspect::ValueViewError::InvalidCast),
								}
							}

							fn into_tag(self) -> crate::Value {
								crate::Value::#name(self)
							}

							fn default_view() -> Self::View {
								&#static_default_name
							}

							fn default_value() -> Self {
								#static_default_name.clone()
							}
						}
					});
				}

				for extra_type in &variant.extra_from
				{
					segments.push(quote! {
						#(#cfg_attributes)*
						impl From<#extra_type> for #enum_name {
							fn from(value: #extra_type) -> Self {
								Self::#name(value as #ty)
							}
						}

						#(#cfg_attributes)*
						impl TryFrom<#enum_name> for #extra_type {
							type Error = ();

							fn try_from(
								value: #enum_name,
							) -> Result<Self, Self::Error> {
								match value {
									#enum_name::#name(value) => {
										<#extra_type>::try_from(value)
											.map_err(|_| ())
									}
									_ => Err(()),
								}
							}
						}
					});
				}

				id_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_name::#name(_) => #id,
				});

				to_kind_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_name::#name(_) => #enum_kind_name::#name,
				});

				from_kind_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name => {
						#enum_name::#name(Default::default())
					},
				});

				kind_name_branches.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name => stringify!(#name),
				});

				kind_variants.push(quote! {
					#(#cfg_attributes)*
					#(#doc_attributes)*
					#name,
				});

				all_kinds.push(quote! {
					#(#cfg_attributes)*
					#enum_kind_name::#name,
				});
			}
		}
	}

	let num_ids = ids.len();

	Ok(quote! {
		impl #enum_name {
			/// Returns the unique ID associated with this variant.
			pub fn id(&self) -> u8 {
				match self {
					#(#id_branches)*
				}
			}

			/// Returns the corresponding kind for this variant.
			pub fn kind(&self) -> #enum_kind_name {
				match self {
					#(#to_kind_branches)*
				}
			}

			/// Returns all possible IDs.
			pub const fn all_ids() -> [u8; #num_ids] {
				[#(#ids)*]
			}
		}

		/// Kinds of an enum, without any associated values.
		#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
		#viz enum #enum_kind_name {
			#(#kind_variants)*
		}

		impl #enum_kind_name {
			/// Returns the string name of the kind.
			pub fn name(&self) -> &'static str {
				match self {
					#(#kind_name_branches)*
				}
			}

			/// Returns a slice containing all kinds.
			pub fn all_kinds() -> &'static [#enum_kind_name] {
				&[#(#all_kinds)*]
			}

			/// Returns the default value for this kind.
			pub fn default_value(&self) -> #enum_name {
				match self {
					#(#from_kind_branches)*
				}
			}
		}

		#(#segments)*
	})
}
