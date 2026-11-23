#![allow(clippy::option_if_let_else)]
use crate::minted_path;
use darling::{
    FromDeriveInput, FromField, FromMeta, FromVariant,
    ast::{Data, Fields},
    util::Override,
};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{
    Error, Expr, GenericArgument, Generics, Ident, Lifetime, LifetimeParam, LitStr, PathArguments,
    Result, Type, Visibility,
};

mod views;

const VIEW_LIFETIME: &str = "__tag_view";

#[derive(Debug, FromDeriveInput)]
#[darling(
    attributes(mint),
    supports(struct_named, enum_named, enum_unit, enum_tuple, enum_newtype)
)]
struct SchemaInput {
    ident: Ident,
    vis: Visibility,
    generics: Generics,
    data: Data<VariantInput, FieldInput>,
    #[darling(default)]
    default: bool,
}

#[derive(Debug, FromField)]
#[darling(attributes(mint))]
struct FieldInput {
    ident: Option<Ident>,
    vis: Visibility,
    ty: Type,
    #[darling(flatten)]
    args: FieldArgs,
}

#[derive(Debug, FromVariant)]
#[darling(attributes(mint))]
struct VariantInput {
    ident: Ident,
    fields: darling::ast::Fields<FieldInput>,
    #[darling(flatten)]
    args: VariantArgs,
}

#[derive(Debug, Default, FromMeta)]
struct VariantArgs {
    #[darling(default)]
    key: Option<String>,
}

#[derive(Debug, Default, FromMeta)]
struct FieldArgs {
    #[darling(default)]
    key: Option<String>,
    #[darling(default)]
    default: Option<Override<DefaultSpec>>,
    #[darling(default)]
    skip: bool,
}

#[derive(Debug, Clone, FromMeta)]
enum DefaultSpec {
    Default,
    #[darling(rename = "from")]
    From(String),
    #[darling(rename = "expr")]
    Expr(Expr),
}

impl FieldArgs {
    fn default(&self, schema_default: bool) -> Option<DefaultSpec> {
        match &self.default {
            None => {
                if schema_default {
                    Some(DefaultSpec::Default)
                } else {
                    None
                }
            }
            Some(Override::Explicit(spec)) => Some(spec.clone()),
            Some(Override::Inherit) => Some(DefaultSpec::Default),
        }
    }
}

#[derive(Debug)]
struct EnumVariant {
    ident: Ident,
    key: String,
    fields: Vec<Field>,
    tuple: bool,
}

#[derive(Debug)]
struct Field {
    ident: Ident,
    vis: Visibility,
    key: String,
    ty: Type,
    default: Option<DefaultSpec>,
    optional: bool,
}

pub fn derive(input: &syn::DeriveInput) -> Result<TokenStream2> {
    let input = SchemaInput::from_derive_input(input)?;
    if input.generics.lifetimes().next().is_some() {
        return Err(Error::new_spanned(
            input.generics,
            "Tag schema source types must not have lifetime parameters",
        ));
    }
    match &input.data {
        Data::Struct(fields) => expand_struct_schema(&input, fields),
        Data::Enum(variants) => expand_enum_schema(&input, variants),
    }
}
fn expand_struct_schema(input: &SchemaInput, fields: &Fields<FieldInput>) -> Result<TokenStream2> {
    let lifetime = Lifetime::new(&format!("'{VIEW_LIFETIME}"), Span::call_site());
    let mut included = Vec::new();
    let mut skipped = Vec::new();
    for field in &fields.fields {
        if field.args.skip {
            let ident = field
                .ident
                .as_ref()
                .expect("named struct field must have an identifier");
            if field.args.key.is_some() {
                return Err(Error::new_spanned(
                    ident,
                    "`key` cannot be used with `skip`",
                ));
            }
            if field.args.default.is_some() {
                return Err(Error::new_spanned(
                    ident,
                    "`default` cannot be used with `skip`",
                ));
            }
            skipped.push(field);
        } else {
            included.push(normalize_field(field, input.default, None)?);
        }
    }
    generate_schema(input, &included, &skipped, &lifetime)
}
fn option_inner(ty: &Type) -> Option<&Type> {
    let path = match ty {
        Type::Path(path) if path.qself.is_none() => Some(path),
        _ => None,
    }?;
    if path.path.segments.len() != 1 {
        return None;
    }
    let segment = path.path.segments.first()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first() {
        Some(GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}
fn normalize_field(
    field: &FieldInput,
    schema_default: bool,
    index: Option<usize>,
) -> Result<Field> {
    if field.ident.is_none() && index.is_none() {
        return Err(Error::new_spanned(
            &field.ty,
            "tuple struct fields must have an index",
        ));
    }
    if field.ident.is_some() && index.is_some() {
        return Err(Error::new_spanned(
            &field.ty,
            "named struct fields must not have an index",
        ));
    }
    let ident = field
        .ident
        .clone()
        .unwrap_or_else(|| format_ident!("p{}", index.unwrap()));
    let vis = field.vis.clone();
    let key = field.args.key.clone().unwrap_or_else(|| ident.to_string());
    let (ty, optional) = option_inner(&field.ty)
        .map(|inner| (inner.clone(), true))
        .unwrap_or_else(|| (field.ty.clone(), false));
    if optional && option_inner(&ty).is_some() {
        return Err(Error::new_spanned(
            &field.ty,
            "nested Option types are not supported",
        ));
    }
    if field.args.default.is_some() && optional {
        return Err(Error::new_spanned(
            &ident,
            "defaults cannot be used on optional fields",
        ));
    }
    Ok(Field {
        ident,
        vis,
        key,
        ty,
        default: field.args.default(schema_default),
        optional,
    })
}
fn normalize_enum_variants(
    variants: &[VariantInput],
    schema_default: bool,
) -> Result<Vec<EnumVariant>> {
    let mut result = Vec::with_capacity(variants.len());
    let mut keys = HashSet::new();
    for variant in variants {
        let key = variant
            .args
            .key
            .clone()
            .unwrap_or_else(|| variant.ident.to_string());
        if !keys.insert(key.clone()) {
            return Err(Error::new_spanned(
                &variant.ident,
                format!("duplicate enum variant key `{key}`"),
            ));
        }
        let (fields, tuple) = match variant.fields.style {
            darling::ast::Style::Unit => (Vec::new(), false),
            darling::ast::Style::Struct => {
                let mut fields = Vec::with_capacity(variant.fields.fields.len());
                for field in &variant.fields.fields {
                    if field.args.skip {
                        return Err(Error::new_spanned(
                            &field.ident,
                            "`skip` is not supported on enum variant fields",
                        ));
                    }
                    let field = normalize_field(field, schema_default, None)?;
                    if field.key == "$type" {
                        return Err(Error::new_spanned(
                            &field.ident,
                            "enum field key `$type` is reserved",
                        ));
                    }
                    fields.push(field);
                }
                (fields, false)
            }
            darling::ast::Style::Tuple => {
                let mut fields = Vec::with_capacity(variant.fields.fields.len());
                for (i, field) in variant.fields.fields.iter().enumerate() {
                    if field.args.skip {
                        return Err(Error::new_spanned(
                            &field.ident,
                            "`skip` is not supported on enum variant fields",
                        ));
                    }
                    let field = normalize_field(field, schema_default, Some(i))?;
                    fields.push(field);
                }
                (fields, true)
            }
        };
        result.push(EnumVariant {
            ident: variant.ident.clone(),
            key,
            fields,
            tuple,
        });
    }
    Ok(result)
}
fn view_type_generics(generics: &Generics, lifetime: &Lifetime) -> TokenStream2 {
    let mut params = Vec::new();
    params.push(quote!(#lifetime));
    for param in &generics.params {
        match param {
            syn::GenericParam::Lifetime(param) => {
                let lifetime = &param.lifetime;
                params.push(quote!(#lifetime));
            }
            syn::GenericParam::Type(param) => {
                let ident = &param.ident;
                params.push(quote!(#ident));
            }
            syn::GenericParam::Const(param) => {
                let ident = &param.ident;
                params.push(quote!(#ident));
            }
        }
    }
    if params.is_empty() {
        quote!()
    } else {
        quote!(<#(#params),*>)
    }
}
fn generate_view_fields(
    name: &Ident,
    fields: &[Field],
    lifetime: &Lifetime,
) -> impl Iterator<Item = TokenStream2> {
    let minted = minted_path();
    fields.iter().map(move |field| {
        let ident = &field.ident;
        let key = &field.key;
        let ty = &field.ty;
        if field.optional
        {
            quote! {
                #ident: match tags.get_raw(#minted::Key::from(#key)) {
                    Some(tag) => Some(
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(tag)?
                    ),
                    None => None,
                }
            }
        }
        else if let Some(DefaultSpec::Expr(_)) = &field.default
        {
            let default = literal_default_view(field, lifetime, name);
            quote! {
                #ident: match tags.get_raw(#minted::Key::from(#key)) {
                    Some(tag) => {
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(tag)?
                    }
                    None => #default,
                }
            }
        }
        else if let Some(DefaultSpec::From(source)) = &field.default
        {
            let Some(source_field) = fields.iter().find(|candidate| candidate.key == *source)
            else {
                return quote! {
                    #ident: {
                        return Err(#minted::introspect::ValueViewError::Missing {
                            key: #minted::Key::from(#source),
                        });
                    }
                };
            };
            let source_default = match &source_field.default
            {
                Some(DefaultSpec::Expr(_)) => literal_default_view(source_field, lifetime, name),
                Some(DefaultSpec::Default) =>
                {
                    quote! {
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::default_view()
                    }
                }
                Some(DefaultSpec::From(_)) | None =>
                {
                    quote! {
                        {
                            return Err(#minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from(#source),
                            });
                        }
                    }
                }
            };
            quote! {
                #ident: match tags.get_raw(#minted::Key::from(#key)) {
                    Some(tag) => {
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(tag)?
                    }
                    None => match tags.get_raw(#minted::Key::from(#source)) {
                        Some(tag) => {
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(tag)?
                        }
                        None => #source_default,
                    },
                }
            }
        }
        else if matches!(&field.default, Some(DefaultSpec::Default)) {
            quote! {
                #ident: match tags.get_raw(#minted::Key::from(#key)) {
                    Some(tag) => {
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(tag)?
                    }
                    None => <#ty as #minted::introspect::IntrospectValue<#lifetime>>::default_view(),
                }
            }
        }
        else {
            quote! {
                #ident: <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(
                    tags.get_raw(#minted::Key::from(#key)).ok_or_else(|| {
                        #minted::introspect::ValueViewError::Missing {
                            key: #minted::Key::from(#key),
                        }
                    })?
                )?
            }
        }
    })
}
fn generate_view_mut_fields(
    fields: &[Field],
    lifetime: &Lifetime,
) -> impl Iterator<Item = TokenStream2> {
    let minted = minted_path();
    fields.iter().enumerate().map(move |(index, field)| {
        let ident = &field.ident;
        let key = &field.key;
        let ty = &field.ty;
        let value = format_ident!("__tag_value_{index}");
        if field.optional {
            quote! {
                #ident: match #value {
                    Some(tag) => Some(
                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view_mut(tag)?
                    ),
                    None => None,
                }
            }
        } else {
            quote! {
                #ident: <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view_mut(
                    #value.ok_or_else(|| #minted::introspect::ValueViewError::Missing {
                        key: #minted::Key::from(#key),
                    })?
                )?
            }
        }
    })
}
fn generate_view_mut_values(fields: &[Field]) -> TokenStream2 {
    let minted = minted_path();
    let bindings = fields
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("__tag_value_{index}"));
    let keys = fields.iter().map(|field| {
        let key = &field.key;
        quote!(&#minted::Key::from(#key))
    });
    quote! {
        let [
            #(#bindings),*
        ] = tags.get_many_mut([
            #(#keys),*
        ]);
    }
}
fn generate_default_insertions(fields: &[Field]) -> impl Iterator<Item = TokenStream2> {
    let minted = minted_path();
    fields.iter().filter_map(move |field| {
        let default = field.default.as_ref()?;
        let key = &field.key;
        let ty = &field.ty;
        Some(match default
        {
			DefaultSpec::Expr(expr) => {
				let expr_value = if is_string(ty) {
					quote!((#expr).to_owned())
				} else {
					quote!(#expr)
				};

				quote! {
					if !tags.contains_key(#minted::Key::from(#key)) {
						let __default_value: #ty = #expr_value;

						tags.insert(
							#minted::Key::from(#key),
							<#ty as #minted::introspect::IntrospectValue<'static>>::into_tag(
								__default_value
							),
						);
					}
				}
			}
            DefaultSpec::From(source) =>
            {
                quote! {
                    if !tags.contains_key(#minted::Key::from(#key)) {
                        let value = tags
                            .get_raw(#minted::Key::from(#source))
                            .ok_or_else(|| #minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from(#source),
                            })?
                            .clone();
                        tags.insert(#minted::Key::from(#key), value);
                    }
                }
            }
            DefaultSpec::Default =>
            {
                quote! {
                    if !tags.contains_key(#minted::Key::from(#key)) {
                        tags.insert(
                            #minted::Key::from(#key),
                            <#ty as #minted::introspect::IntrospectValue<'static>>::into_tag(
                                <#ty as #minted::introspect::IntrospectValue<'static>>::default_value()
                            ),
                        );
                    }
                }
            }
        })
    })
}
fn literal_default_owned(name: &Ident, field: &Field, lifetime: &Lifetime) -> TokenStream2 {
    let minted = minted_path();
    let ty = &field.ty;
    let default_ident = format_ident!("__TAG_DEFAULT_{}_{}", name, field.ident);
    quote! {
        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::from_tag(
            &*#default_ident
        )?
    }
}
fn generate_from_tags_fields(
    name: &Ident,
    fields: &[Field],
    lifetime: &Lifetime,
) -> Result<TokenStream2> {
    let minted = minted_path();
    let generated = fields
        .iter()
        .map(|field| {
            let ident = &field.ident;
            let key = &field.key;
            let ty = &field.ty;
            if field.optional
            {
                Ok(quote! {
                    #ident: match tags.get_raw(#minted::Key::from(#key)) {
                        Some(tag) => Some(
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                ::from_tag(tag)?
                        ),
                        None => None,
                    }
                })
            }
            else {
                match &field.default
                {
                    Some(DefaultSpec::Expr(expr)) =>
                    {
                        let static_decl = literal_default_static(name, field, expr);
                        let default_value = literal_default_owned(name, field, lifetime);
                        Ok(quote! {
                            #ident: {
                                #static_decl
                                match tags.get_raw(#minted::Key::from(#key)) {
                                    Some(tag) => {
                                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                            ::from_tag(tag)?
                                    }
                                    None => #default_value,
                                }
                            }
                        })
                    }
                    Some(DefaultSpec::From(source)) =>
                    {
                        let source_field =
                            fields
                                .iter()
                                .find(|candidate| candidate.key == *source)
                                .ok_or_else(|| {
                                    Error::new_spanned(
                                        &field.ident,
                                        format!("default source `{source}` does not exist"),
                                    )
                                })?;
                        let source_fallback = match &source_field.default
                        {
                            Some(DefaultSpec::Expr(expr)) =>
                            {
                                let static_decl = literal_default_static(name, source_field, expr);
                                let default_value = literal_default_owned(name, source_field, lifetime);
                                quote! {
                                    {
                                        #static_decl
                                        #default_value
                                    }
                                }
                            }
                            Some(DefaultSpec::From(_)) | None =>
                            {
                                quote! {
                                    {
                                        return Err(
                                            #minted::introspect::ValueViewError::Missing {
                                                key: #minted::Key::from(#source),
                                            }
                                        );
                                    }
                                }
                            }
                            Some(DefaultSpec::Default) =>
                            {
                                quote! {
                                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>::default_value()
                                }
                            }
                        };
                        Ok(quote! {
                            #ident: match tags.get_raw(#minted::Key::from(#key)) {
                                Some(tag) => {
                                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                        ::from_tag(tag)?
                                }
                                None => match tags.get_raw(#minted::Key::from(#source)) {
                                    Some(tag) => {
                                        <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                            ::from_tag(tag)?
                                    }
                                    None => #source_fallback,
                                },
                            }
                        })
                    }
                    Some(DefaultSpec::Default) =>
                    {
                        Ok(quote! {
                            #ident: match tags.get_raw(#minted::Key::from(#key)) {
                                Some(tag) => {
                                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                        ::from_tag(tag)?
                                }
                                None => <#ty as #minted::introspect::IntrospectValue<#lifetime>>::default_value(),
                            }
                        })
                    }
                    None => Ok(quote! {
                        #ident:
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                ::from_tag(
                                    tags.get_raw(#minted::Key::from(#key)).ok_or_else(|| {
                                        #minted::introspect::ValueViewError::Missing {
                                            key: #minted::Key::from(#key),
                                        }
                                    })?
                                )?
                    }),
                }
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(quote! {
        #(#generated),*
    })
}
fn generate_into_tags_fields(fields: &[Field]) -> impl Iterator<Item = TokenStream2> {
    let minted = minted_path();
    fields.iter().map(move |field| {
        let ident = &field.ident;
        let key = &field.key;
        let ty = &field.ty;
        if field.optional {
            quote! {
                if let Some(value) = self.#ident {
                    tags.insert(
                        #minted::Key::from(#key),
                        <#ty as #minted::introspect::IntrospectValue<'static>>
                            ::into_tag(value),
                    );
                }
            }
        } else {
            quote! {
                tags.insert(
                    #minted::Key::from(#key),
                    <#ty as #minted::introspect::IntrospectValue<'static>>
                        ::into_tag(self.#ident),
                );
            }
        }
    })
}
fn is_string(ty: &Type) -> bool {
    let path = match ty {
        Type::Path(path) if path.qself.is_none() => path,
        _ => return false,
    };
    if path.path.segments.len() != 1 {
        return false;
    }
    path.path
        .segments
        .first()
        .map(|segment| &segment.ident)
        .is_some_and(|ident| ident == "String")
}
fn literal_default_static(name: &Ident, field: &Field, expr: &Expr) -> TokenStream2 {
    let ident = &field.ident;
    let default_ident = format_ident!("__TAG_DEFAULT_{}_{}", name, ident);
    let minted = minted_path();
    let ty = &field.ty;
    let expr_value = if is_string(ty) {
        quote!((#expr).to_owned())
    } else {
        quote!(#expr)
    };

    quote! {
        #[allow(nonstandard_style)]
        static #default_ident: ::std::sync::LazyLock<#minted::Value> =
            ::std::sync::LazyLock::new(|| {
                <#ty as #minted::introspect::IntrospectValue<'static>>::into_tag(
                    #expr_value
                )
            });
    }
}
fn literal_default_view(field: &Field, lifetime: &Lifetime, name: &Ident) -> TokenStream2 {
    let default_ident = format_ident!("__TAG_DEFAULT_{}_{}", name, field.ident);
    let minted = minted_path();
    let ty = &field.ty;
    quote! {
        <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(
            &*#default_ident
        )?
    }
}
fn generate_schema(
    input: &SchemaInput,
    fields: &[Field],
    skipped_fields: &[&FieldInput],
    lifetime: &Lifetime,
) -> Result<TokenStream2> {
    let view = views::generate_views(input, lifetime, views::ViewInput::Struct { fields })?;
    let name = &input.ident;
    let generics = &input.generics;
    let minted = minted_path();
    let view_name = format_ident!("{name}View");
    let view_mut_name = format_ident!("{name}ViewMut");
    let mut view_generics = generics.clone();
    view_generics.params.insert(
        0,
        syn::GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())),
    );
    let view_ty_generics = view_type_generics(generics, lifetime);
    let (view_impl_generics, _, _) = view_generics.split_for_impl();
    let (_, owned_ty_generics, owned_where) = generics.split_for_impl();
    let default_statics = fields.iter().filter_map(|field| match &field.default {
        Some(DefaultSpec::Expr(expr)) => Some(literal_default_static(name, field, expr)),
        _ => None,
    });
    let view_fields = generate_view_fields(name, fields, lifetime).collect::<Vec<_>>();
    let view_mut_values = generate_view_mut_values(fields);
    let view_mut_fields = generate_view_mut_fields(fields, lifetime).collect::<Vec<_>>();
    let default_insertions = generate_default_insertions(fields).collect::<Vec<_>>();
    let from_tags_fields = generate_from_tags_fields(name, fields, lifetime)?;
    let into_tags_fields = generate_into_tags_fields(fields).collect::<Vec<_>>();
    let into_fields_len = into_tags_fields.len();
    let skipped_initializers = skipped_fields.iter().map(|field| {
        let ident = field
            .ident
            .as_ref()
            .expect("named struct field must have an identifier");
        quote! {
            #ident: ::core::default::Default::default(),
        }
    });
    Ok(quote! {
        #(#default_statics)*
        #view
        impl #view_impl_generics
            #minted::introspect::Introspect<#lifetime>
            for #name #owned_ty_generics
            #owned_where
        {
            type View = #view_name #view_ty_generics;
            type ViewMut = #view_mut_name #view_ty_generics;
            fn view(
                tags: &#lifetime #minted::Values,
            ) -> ::core::result::Result<
                Self::View,
                #minted::introspect::ValueViewError,
            > {
                Ok(#view_name {
                    #(#view_fields,)*
                })
            }
            fn view_mut(
                tags: &'__tag_view mut #minted::Values,
            ) -> ::core::result::Result<
                Self::ViewMut,
                #minted::introspect::ValueViewError,
            > {
                #(#default_insertions)*
                #view_mut_values
                Ok(#view_mut_name {
                    #(#view_mut_fields,)*
                })
            }
            fn from_tags(
                tags: &#lifetime #minted::Values,
            ) -> ::core::result::Result<
                Self,
                #minted::introspect::ValueViewError,
            > {
                Ok(Self {
                    #(#skipped_initializers)*
                    #from_tags_fields
                })
            }
            fn into_tags(self) -> #minted::Values {
                let mut tags =
                    #minted::Values::with_capacity(#into_fields_len);
                #(#into_tags_fields)*
                tags
            }
        }
    })
}
/* ------------------------------------------------------------------------- */
/* Enum generation                                                          */
/* ------------------------------------------------------------------------- */
fn expand_enum_schema(input: &SchemaInput, variants: &[VariantInput]) -> Result<TokenStream2> {
    let variants = normalize_enum_variants(variants, input.default)?;
    if variants.is_empty() {
        return Err(Error::new_spanned(
            &input.ident,
            "empty enums are not supported",
        ));
    }
    let has_unit = variants.iter().any(|variant| variant.fields.is_empty());
    let has_data = variants.iter().any(|variant| !variant.fields.is_empty());
    let has_tupl = variants.iter().any(|variant| variant.tuple);
    let all_tupl = variants.iter().all(|variant| variant.tuple);
    if (has_unit && has_data) || (has_unit && has_tupl) || (has_tupl && !all_tupl) {
        return Err(Error::new_spanned(
            &input.ident,
            "enums must contain all of the same kind of variant",
        ));
    }
    if has_unit {
        generate_unit_enum_schema(input, &variants)
    } else {
        generate_named_enum_schema(input, &variants, all_tupl)
    }
}
fn generate_unit_enum_schema(
    input: &SchemaInput,
    variants: &[EnumVariant],
) -> Result<TokenStream2> {
    let name = &input.ident;
    let generics = &input.generics;
    let minted = minted_path();
    let lifetime = Lifetime::new(&format!("'{VIEW_LIFETIME}"), Span::call_site());
    let mut impl_generics = generics.clone();
    impl_generics.params.insert(
        0,
        syn::GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())),
    );
    let (impl_generics, _, where_clause) = impl_generics.split_for_impl();
    let (_, ty_generics, _) = generics.split_for_impl();
    let match_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let key = &variant.key;
        quote! {
            #key => Ok(Self::#ident)
        }
    });
    let into_arms = variants.iter().map(|variant| {
        let ident = &variant.ident;
        let key = &variant.key;
        quote! {
            Self::#ident => #minted::Value::String(
                #key.to_owned()
            )
        }
    });
    Ok(quote! {
        impl #impl_generics
            #minted::introspect::IntrospectValue<#lifetime>
            for #name #ty_generics
            #where_clause
        {
            type View = Self;
            type ViewMut = #minted::introspect::UnitEnum<#lifetime, Self>;
            fn view(
                tag: &#lifetime #minted::Value,
            ) -> ::core::result::Result<
                Self::View,
                #minted::introspect::ValueViewError,
            > {
                match tag {
                    #minted::Value::String(value) => {
                        match value.as_str() {
                            #(#match_arms,)*
                            _ => Err(
                                #minted::introspect::ValueViewError::WrongType {
                                    key: #minted::Key::from("root"),
                                }
                            ),
                        }
                    }
                    _ => Err(
                        #minted::introspect::ValueViewError::WrongType {
                            key: #minted::Key::from("root"),
                        }
                    ),
                }
            }
            fn view_mut(
                tag: &#lifetime mut #minted::Value,
            ) -> ::core::result::Result<
                Self::ViewMut,
                #minted::introspect::ValueViewError,
            > {
                Ok(#minted::introspect::UnitEnum::new(tag))
            }
            fn from_tag(
                tag: &#lifetime #minted::Value,
            ) -> ::core::result::Result<
                Self,
                #minted::introspect::ValueViewError,
            > {
                Self::view(tag)
            }
            fn into_tag(self) -> #minted::Value {
                match self {
                    #(#into_arms,)*
                }
            }
            fn default_view() -> Self::View {
                Self::default()
            }
            fn default_value() -> Self {
                Self::default()
            }
        }
    })
}
fn generate_named_enum_schema(
    input: &SchemaInput,
    variants: &[EnumVariant],
    is_tupl: bool,
) -> Result<TokenStream2> {
    let lifetime = Lifetime::new(&format!("'{VIEW_LIFETIME}"), Span::call_site());
    let view = views::generate_views(
        input,
        &lifetime,
        views::ViewInput::Enum { variants, is_tupl },
    )?;
    let name = &input.ident;
    let generics = &input.generics;
    let minted = minted_path();
    let lifetime = Lifetime::new(&format!("'{VIEW_LIFETIME}"), Span::call_site());
    let view_name = format_ident!("{name}View");
    let view_mut_name = format_ident!("{name}ViewMut");
    let mut view_generics = generics.clone();
    view_generics.params.insert(
        0,
        syn::GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())),
    );
    let view_ty_generics = view_type_generics(generics, &lifetime);
    let (view_impl_generics, _, _) = view_generics.split_for_impl();
    let (_, owned_ty_generics, owned_where) = generics.split_for_impl();
    let view_arms = generate_enum_view_arms(variants, &view_name, &lifetime, is_tupl)?;
    let view_mut_arms = generate_enum_view_mut_arms(variants, &view_mut_name, &lifetime, is_tupl)?;
    let from_tag_arms = generate_enum_from_tag_arms(variants, &lifetime, is_tupl)?;
    let into_tag_arms = generate_enum_into_tag_arms(variants, is_tupl)?;
    Ok(quote! {
        #view
        impl #view_impl_generics
            #minted::introspect::Introspect<#lifetime>
            for #name #owned_ty_generics
            #owned_where
        {
            type View = #view_name #view_ty_generics;
            type ViewMut = #view_mut_name #view_ty_generics;
            fn view(
                tags: &#lifetime #minted::Values,
            ) -> ::core::result::Result<Self::View, #minted::introspect::ValueViewError> {
                let variant = match tags.get_raw(#minted::Key::from("$type")) {
                    Some(#minted::Value::String(value)) => value.as_str(),
                    Some(_) => {
                        return Err(
                            #minted::introspect::ValueViewError::WrongType {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                    None => {
                        return Err(
                            #minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                };
                match variant {
                    #(#view_arms,)*
                    _ => Err(
                        #minted::introspect::ValueViewError::WrongType {
                            key: #minted::Key::from("$type"),
                        }
                    ),
                }
            }
            fn view_mut(
                tags: &'__tag_view mut #minted::Values,
            ) -> ::core::result::Result<
                Self::ViewMut,
                #minted::introspect::ValueViewError,
            > {
                let variant = match tags.get_raw(#minted::Key::from("$type")) {
                    Some(#minted::Value::String(value)) => value.clone(),
                    Some(_) => {
                        return Err(
                            #minted::introspect::ValueViewError::WrongType {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                    None => {
                        return Err(
                            #minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                };
                match variant.as_str() {
                    #(#view_mut_arms,)*
                    _ => Err(
                        #minted::introspect::ValueViewError::WrongType {
                            key: #minted::Key::from("$type"),
                        }
                    ),
                }
            }
            fn from_tags(
                tags: &#lifetime #minted::Values,
            ) -> ::core::result::Result<Self, #minted::introspect::ValueViewError> {
                let variant = match tags.get_raw(#minted::Key::from("$type")) {
                    Some(#minted::Value::String(value)) => value.as_str(),
                    Some(_) => {
                        return Err(
                            #minted::introspect::ValueViewError::WrongType {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                    None => {
                        return Err(
                            #minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from("$type"),
                            }
                        );
                    }
                };
                match variant {
                    #(#from_tag_arms,)*
                    _ => Err(
                        #minted::introspect::ValueViewError::WrongType {
                            key: #minted::Key::from("$type"),
                        }
                    ),
                }
            }
            fn into_tags(self) -> #minted::Values {
                let mut tags =
                    #minted::Values::with_capacity(1);
                match self {
                    #(#into_tag_arms,)*
                }
                tags
            }
        }
    })
}
fn generate_enum_view_arms(
    variants: &[EnumVariant],
    view_name: &Ident,
    lifetime: &Lifetime,
    is_tupl: bool,
) -> Result<Vec<TokenStream2>> {
    variants
        .iter()
        .map(|variant| {
            let key = LitStr::new(&variant.key, variant.ident.span());
            let fields = generate_enum_view_field_initializers(&variant.fields, lifetime, is_tupl);
            if is_tupl {
                let ident = &variant.ident;
                Ok(quote! {
                    #key => {
                        Ok(#view_name::#ident (
                            #(#fields,)*
                        ))
                    }
                })
            } else {
                let ident = &variant.ident;
                let fields =
                    generate_enum_view_field_initializers(&variant.fields, lifetime, is_tupl);
                Ok(quote! {
                    #key => {
                        Ok(#view_name::#ident {
                            #(#fields,)*
                        })
                    }
                })
            }
        })
        .collect()
}
fn generate_enum_view_mut_arms(
    variants: &[EnumVariant],
    view_mut_name: &Ident,
    lifetime: &Lifetime,
    is_tupl: bool,
) -> Result<Vec<TokenStream2>> {
    let minted = minted_path();
    variants
        .iter()
        .map(|variant| {
            let key = LitStr::new(&variant.key, variant.ident.span());
            let ident = &variant.ident;
            let bindings = variant
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("__tag_value_{index}"))
                .collect::<Vec<_>>();
            let keys = variant.fields.iter().map(|field| {
                let key = &field.key;
                quote!(&#minted::Key::from(#key))
            });
            let fields =
                generate_enum_view_mut_field_initializers(&variant.fields, lifetime, is_tupl);
            let destructure = if bindings.is_empty() {
                quote! {}
            } else {
                quote! {
                    let [#(#bindings),*] =
                        tags.get_many_mut([#(#keys),*]);
                }
            };
            if is_tupl {
                Ok(quote! {
                    #key => {
                        #destructure
                        Ok(#view_mut_name::#ident (
                            #(#fields,)*
                        ))
                    }
                })
            } else {
                Ok(quote! {
                    #key => {
                        #destructure
                        Ok(#view_mut_name::#ident {
                            #(#fields,)*
                        })
                    }
                })
            }
        })
        .collect()
}
fn generate_enum_from_tag_arms(
    variants: &[EnumVariant],
    lifetime: &Lifetime,
    is_tupl: bool,
) -> Result<Vec<TokenStream2>> {
    let minted = minted_path();
    variants
        .iter()
        .map(|variant| {
            let key = LitStr::new(&variant.key, variant.ident.span());
            let ident = &variant.ident;
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    let field_ident = &field.ident;
                    let field_key = &field.key;
                    let ty = &field.ty;
                    let value = if field.optional {
                        quote! {
                            match tags.get_raw(#minted::Key::from(#field_key)) {
                                Some(tag) => Some(
                                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                        ::from_tag(tag)?
                                ),
                                None => None,
                            }
                        }
                    } else {
                        quote! {
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                ::from_tag(
                                    tags.get_raw(#minted::Key::from(#field_key))
                                        .ok_or_else(|| {
                                            #minted::introspect::ValueViewError::Missing {
                                                key: #minted::Key::from(#field_key),
                                            }
                                        })?
                                )?
                        }
                    };
                    if is_tupl {
                        quote! {
                            #value
                        }
                    } else {
                        quote! {
                            #field_ident: #value
                        }
                    }
                })
                .collect::<Vec<_>>();
            if is_tupl {
                Ok(quote! {
                    #key => {
                        Ok(Self::#ident (
                            #(#fields,)*
                        ))
                    }
                })
            } else {
                Ok(quote! {
                    #key => {
                        Ok(Self::#ident {
                            #(#fields,)*
                        })
                    }
                })
            }
        })
        .collect()
}
fn generate_enum_into_tag_arms(
    variants: &[EnumVariant],
    is_tupl: bool,
) -> Result<Vec<TokenStream2>> {
    let minted = minted_path();
    variants
        .iter()
        .map(|variant| {
            let ident = &variant.ident;
            let key = LitStr::new(&variant.key, variant.ident.span());
            let field_idents = variant
                .fields
                .iter()
                .map(|field| &field.ident)
                .collect::<Vec<_>>();
            let insertions = variant
                .fields
                .iter()
                .map(|field| {
                    let field_ident = &field.ident;
                    let field_key = &field.key;
                    let ty = &field.ty;
                    if is_tupl {
                        if field.optional {
                            quote! {
                                if let Some(value) = #field_ident {
                                    tags.insert(
                                        #minted::Key::from(#field_key),
                                        <#ty as #minted::introspect::IntrospectValue<'static>>
                                            ::into_tag(value),
                                    );
                                }
                            }
                        } else {
                            quote! {
                                tags.insert(
                                    #minted::Key::from(#field_key),
                                    <#ty as #minted::introspect::IntrospectValue<'static>>
                                        ::into_tag(#field_ident),
                                );
                            }
                        }
                    } else if field.optional {
                        quote! {
                            if let Some(value) = #field_ident {
                                tags.insert(
                                    #minted::Key::from(#field_key),
                                    <#ty as #minted::introspect::IntrospectValue<'static>>
                                        ::into_tag(value),
                                );
                            }
                        }
                    } else {
                        quote! {
                            tags.insert(
                                #minted::Key::from(#field_key),
                                <#ty as #minted::introspect::IntrospectValue<'static>>
                                    ::into_tag(#field_ident),
                            );
                        }
                    }
                })
                .collect::<Vec<_>>();
            if is_tupl {
                Ok(quote! {
                    Self::#ident (
                        #(#field_idents),*
                    ) => {
                        tags.insert(#minted::Key::from("$type"), #key);
                        #(#insertions)*
                    }
                })
            } else {
                Ok(quote! {
                    Self::#ident {
                        #(#field_idents),*
                    } => {
                        tags.insert(#minted::Key::from("$type"), #key);
                        #(#insertions)*
                    }
                })
            }
        })
        .collect()
}
fn generate_enum_view_field_initializers(
    fields: &[Field],
    lifetime: &Lifetime,
    is_tupl: bool,
) -> Vec<TokenStream2> {
    let minted = minted_path();
    fields
        .iter()
        .map(|field| {
            let ident = &field.ident;
            let key = &field.key;
            let ty = &field.ty;
            let value = if field.optional {
                quote! {
                    match tags.get_raw(#minted::Key::from(#key)) {
                        Some(tag) => Some(
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                ::view(tag)?
                        ),
                        None => None,
                    }
                }
            } else {
                quote! {
                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view(
                        tags.get_raw(#minted::Key::from(#key))
                            .ok_or_else(|| {
                                #minted::introspect::ValueViewError::Missing {
                                    key: #minted::Key::from(#key),
                                }
                            })?
                    )?
                }
            };
            if is_tupl {
                quote! {
                    #value
                }
            } else {
                quote! {
                    #ident: #value
                }
            }
        })
        .collect()
}
fn generate_enum_view_mut_field_initializers(
    fields: &[Field],
    lifetime: &Lifetime,
    is_tupl: bool,
) -> Vec<TokenStream2> {
    let minted = minted_path();
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let ident = &field.ident;
            let key = &field.key;
            let ty = &field.ty;
            let binding = format_ident!("__tag_value_{index}");
            let value = if field.optional {
                quote! {
                    match #binding {
                        Some(tag) => Some(
                            <#ty as #minted::introspect::IntrospectValue<#lifetime>>
                                ::view_mut(tag)?
                        ),
                        None => None,
                    }
                }
            } else {
                quote! {
                    <#ty as #minted::introspect::IntrospectValue<#lifetime>>::view_mut(
                        #binding.ok_or_else(|| {
                            #minted::introspect::ValueViewError::Missing {
                                key: #minted::Key::from(#key),
                            }
                        })?
                    )?
                }
            };
            if is_tupl {
                quote! {
                    #value
                }
            } else {
                quote! {
                    #ident: #value
                }
            }
        })
        .collect()
}
