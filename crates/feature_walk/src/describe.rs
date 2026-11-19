//! `impl Describe`: a struct's fields or an enum's forms, each field's
//! kind from its walk role (a length, an angle...) or else its type's own
//! description, its name and default from serde's attributes.

use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, DeriveInput, Expr, ExprLit, Fields, Lit, Meta, Token};

use crate::{serde_has, serde_rename};

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let root = quote!(::core_document::describe);
    let body = match &input.data {
        Data::Struct(data) => {
            let defaulted = serde_has(&input.attrs, "default")?;
            match &data.fields {
                Fields::Named(_) => {
                    let fields = fields(&data.fields, defaulted)?;
                    let defaults = if defaulted {
                        quote! { let defaults = #root::json(&<Self as ::core::default::Default>::default()); }
                    } else {
                        quote! {}
                    };
                    quote! { #defaults #root::Shape::Fields(vec![#(#fields),*]) }
                }
                Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                    let field = &unnamed.unnamed[0];
                    shape(&field.attrs, &field.ty)?
                }
                Fields::Unnamed(_) => {
                    quote! { #root::Shape::List(Box::new(#root::Shape::Unknown)) }
                }
                Fields::Unit => quote! { #root::Shape::Words("nothing") },
            }
        }
        Data::Enum(data) => {
            let untagged = serde_has(&input.attrs, "untagged")?;
            let mut forms = Vec::new();
            for variant in &data.variants {
                if serde_has(&variant.attrs, "skip")?
                    || serde_has(&variant.attrs, "skip_deserializing")?
                {
                    continue;
                }
                let held = match &variant.fields {
                    Fields::Unit => None,
                    Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                        let field = &unnamed.unnamed[0];
                        Some(shape(&field.attrs, &field.ty)?)
                    }
                    Fields::Unnamed(_) => {
                        Some(quote! { #root::Shape::List(Box::new(#root::Shape::Unknown)) })
                    }
                    Fields::Named(_) => {
                        let fields = fields(&variant.fields, false)?;
                        Some(quote! { #root::Shape::Fields(vec![#(#fields),*]) })
                    }
                };
                let tag =
                    serde_rename(&variant.attrs)?.unwrap_or_else(|| variant.ident.to_string());
                forms.push(if untagged {
                    held.unwrap_or_else(|| quote! { #root::Shape::Words("nothing") })
                } else {
                    let held = match held {
                        Some(held) => quote! { Some(#held) },
                        None => quote! { None },
                    };
                    quote! { #root::Variant { name: #tag.to_string(), shape: #held } }
                });
            }
            if untagged {
                quote! { #root::Shape::OneOf(vec![#(#forms),*]) }
            } else {
                quote! { #root::Shape::Variants(vec![#(#forms),*]) }
            }
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(name, "a union cannot be described"));
        }
    };
    Ok(quote! {
        impl #impl_generics #root::Describe for #name #type_generics #where_clause {
            fn shape() -> #root::Shape {
                #body
            }
        }
    })
}

/// Each named field as a `Field`; `defaulted` when the container's
/// `#[serde(default)]` gives every field its `Default`'s value, read from
/// `defaults`.
fn fields(fields: &Fields, defaulted: bool) -> syn::Result<Vec<Tokens>> {
    let root = quote!(::core_document::describe);
    let mut out = Vec::new();
    for field in fields {
        let Some(ident) = &field.ident else { continue };
        if serde_has(&field.attrs, "skip")? || serde_has(&field.attrs, "skip_deserializing")? {
            continue;
        }
        let name = serde_rename(&field.attrs)?.unwrap_or_else(|| ident.to_string());
        let ty = &field.ty;
        let flatten = serde_has(&field.attrs, "flatten")?;
        let shape = if flatten {
            quote! { <#ty as #root::Describe>::shape() }
        } else {
            shape(&field.attrs, ty)?
        };
        let default = match serde_default(&field.attrs)? {
            Some(None) => quote! { #root::json(&<#ty as ::core::default::Default>::default()) },
            Some(Some(path)) => {
                let path: syn::ExprPath = syn::parse_str(&path)?;
                quote! { #root::json(&#path()) }
            }
            None if flatten => quote! { None },
            // A field serde leaves out of the default's JSON is its own
            // type's default there.
            None if defaulted && serde_has(&field.attrs, "skip_serializing_if")? => quote! {
                defaults.as_ref().and_then(|d| d.get(#name).cloned())
                    .or_else(|| #root::json(&<#ty as ::core::default::Default>::default()))
            },
            None if defaulted => quote! { defaults.as_ref().and_then(|d| d.get(#name).cloned()) },
            None if is_option(ty) => quote! { Some(#root::Value::Null) },
            None => quote! { None },
        };
        out.push(quote! {
            #root::Field {
                name: #name.to_string(),
                shape: #shape,
                default: #default,
                flatten: #flatten,
            }
        });
    }
    Ok(out)
}

/// What a field of type `ty` is written as: a number of the kind its walk
/// role names, else what its type describes.
fn shape(attrs: &[Attribute], ty: &syn::Type) -> syn::Result<Tokens> {
    let root = quote!(::core_document::describe);
    let typed = quote! { <#ty as #root::Describe>::shape() };
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident("walk")) else {
        return Ok(typed);
    };
    let Meta::List(list) = attr.parse_args::<Meta>()? else {
        return Ok(typed);
    };
    let role = list
        .path
        .get_ident()
        .map(ToString::to_string)
        .unwrap_or_default();
    let number = |kind: &str| quote! { #root::Shape::Number(#kind) };
    Ok(match role.as_str() {
        "length" | "angle" | "number" | "count" => {
            let number = number(&role);
            if is_option(ty) {
                quote! { #root::Shape::Optional(Box::new(#number)) }
            } else {
                number
            }
        }
        "lengths" | "angles" | "numbers" | "counts" => {
            let number = number(role.trim_end_matches('s'));
            quote! { #root::Shape::List(Box::new(#number)) }
        }
        "axes" => {
            let args =
                Punctuated::<Expr, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
            let kind = match args.get(2) {
                Some(Expr::Path(p)) => p.path.get_ident().map(ToString::to_string),
                Some(Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                })) => Some(s.value()),
                _ => None,
            }
            .unwrap_or_else(|| "number".into());
            let number = number(&kind);
            quote! { #root::Shape::Array(3, Box::new(#number)) }
        }
        _ => typed,
    })
}

/// `#[serde(default)]` as `Some(None)`, `#[serde(default = "f")]` as
/// `Some(Some("f"))`, none as `None`.
fn serde_default(attrs: &[Attribute]) -> syn::Result<Option<Option<String>>> {
    let mut found = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                found = Some(if meta.input.peek(Token![=]) {
                    let lit: syn::LitStr = meta.value()?.parse()?;
                    Some(lit.value())
                } else {
                    None
                });
            } else if meta.input.peek(Token![=]) {
                meta.value()?.parse::<Expr>()?;
            } else if meta.input.peek(syn::token::Paren) {
                meta.parse_nested_meta(|_| Ok(()))?;
            }
            Ok(())
        })?;
    }
    Ok(found)
}

/// Whether `ty` is an `Option<...>`, which serde leaves `None` when the
/// field is left out.
fn is_option(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option"))
}
