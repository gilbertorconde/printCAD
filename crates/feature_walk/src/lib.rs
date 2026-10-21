//! `#[derive(Walk)]` for `core_document::walk::Walk`: each field walked
//! by the role its `#[walk(...)]` attribute names, or by its own type's
//! `Walk` when it names none, at the place serde writes it. The roles are
//! described in `core_document::walk`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Fields, Lit, Meta, Token, parse_macro_input,
};

#[proc_macro_derive(Walk, attributes(walk))]
pub fn derive_walk(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    // A type of settings alone walks nothing.
    let mut names = Vec::new();
    let body = if container_is_setting(&input.attrs)? {
        quote! {}
    } else {
        match &input.data {
            Data::Struct(data) => {
                let (pattern, steps) = fields(&data.fields, quote!(Self), &mut names)?;
                quote! { let #pattern = self; #(#steps)* }
            }
            Data::Enum(data) => {
                let untagged = serde_has(&input.attrs, "untagged")?;
                let mut arms = Vec::new();
                for variant in &data.variants {
                    let ident = &variant.ident;
                    let (pattern, steps) =
                        fields(&variant.fields, quote!(Self::#ident), &mut names)?;
                    let tag = serde_rename(&variant.attrs)?.unwrap_or_else(|| ident.to_string());
                    arms.push(if untagged || steps.is_empty() {
                        quote! { #pattern => { #(#steps)* } }
                    } else {
                        quote! { #pattern => walker.at(#tag, |walker| { #(#steps)* }), }
                    });
                }
                quote! {
                    #[allow(unused_variables)]
                    match self { #(#arms)* }
                }
            }
            Data::Union(_) => {
                return Err(syn::Error::new_spanned(name, "a union cannot be walked"));
            }
        }
    };
    Ok(quote! {
        impl #impl_generics ::core_document::walk::Walk for #name #type_generics #where_clause {
            #[allow(unused_variables)]
            fn walk(&self, walker: &mut ::core_document::walk::Walker) {
                #body
            }

            fn names(out: &mut ::std::vec::Vec<&'static str>) {
                #(#names)*
            }
        }
    })
}

/// The pattern binding a struct's or variant's fields, and what walks
/// each.
fn fields(
    fields: &Fields,
    path: Tokens,
    names: &mut Vec<Tokens>,
) -> syn::Result<(Tokens, Vec<Tokens>)> {
    let mut steps = Vec::new();
    let mut bindings = Vec::new();
    let single = fields.len() == 1;
    for (i, field) in fields.iter().enumerate() {
        let binding = format_ident!("field_{i}");
        let segment = match &field.ident {
            Some(ident) => {
                bindings.push(quote!(#ident: #binding));
                Some(serde_rename(&field.attrs)?.unwrap_or_else(|| ident.to_string()))
            }
            None => {
                bindings.push(quote!(#binding));
                // A newtype variant's one field is the variant's value; a
                // tuple's fields are its items.
                (!single).then(|| i.to_string())
            }
        };
        let segment = if serde_has(&field.attrs, "flatten")? {
            None
        } else {
            segment
        };
        let ty = &field.ty;
        let Some((walk, named)) = role(&field.attrs, &binding)? else {
            continue;
        };
        names.push(match named {
            Named::Names(list) => quote! { #(out.push(#list);)* },
            Named::Nested => quote! { <#ty as ::core_document::walk::Walk>::names(out); },
            Named::Items => steps_names(ty),
            Named::Unknown => quote! {},
        });
        steps.push(match segment {
            Some(segment) => quote! { walker.at(#segment, |walker| { #walk }); },
            None => quote! { #walk },
        });
    }
    let pattern = match fields {
        Fields::Named(_) => quote! { #path { #(#bindings,)* } },
        Fields::Unnamed(_) => quote! { #path ( #(#bindings,)* ) },
        Fields::Unit => quote! { #path },
    };
    Ok((pattern, steps))
}

/// The names a field gives its numbers: listed, its own type's, its
/// list items' type's, or not known until walked (a `with` function).
enum Named {
    Names(Vec<String>),
    Nested,
    Items,
    Unknown,
}

/// The names of a `steps` list's items: those of `Vec<T>`'s `T`.
fn steps_names(ty: &syn::Type) -> Tokens {
    if let syn::Type::Path(path) = ty
        && let Some(last) = path.path.segments.last()
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(item)) = args.args.first()
    {
        return quote! { <#item as ::core_document::walk::Walk>::names(out); };
    }
    quote! {}
}

/// What walks a field bound as `binding`, by its `#[walk(...)]`, and the
/// names it gives; `None` for a setting.
fn role(attrs: &[Attribute], binding: &syn::Ident) -> syn::Result<Option<(Tokens, Named)>> {
    let mut found = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("walk")) {
        if found.is_some() {
            return Err(syn::Error::new_spanned(attr, "a field takes one role"));
        }
        let meta: Meta = attr.parse_args()?;
        found = Some(match &meta {
            Meta::Path(path) if path.is_ident("setting") => None,
            Meta::NameValue(nv) if nv.path.is_ident("with") => {
                let Expr::Path(func) = &nv.value else {
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "with = a function's path",
                    ));
                };
                Some((quote! { #func(#binding, walker); }, Named::Unknown))
            }
            Meta::List(list) => {
                let kind = list
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let args =
                    Punctuated::<Expr, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
                let text = |i: usize| -> syn::Result<String> {
                    match args.get(i) {
                        Some(Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        })) => Ok(s.value()),
                        _ => Err(syn::Error::new_spanned(list, "expected a name and a label")),
                    }
                };
                let (name, label) = (text(0)?, text(1)?);
                match kind.as_str() {
                    "length" | "angle" | "number" | "count" => {
                        let dim = dim(&kind);
                        let walk = quote! { walker.number(#binding, #name, #label, #dim); };
                        Some((walk, Named::Names(vec![name])))
                    }
                    "lengths" | "angles" | "numbers" | "counts" => {
                        let dim = dim(kind.trim_end_matches('s'));
                        let walk = quote! { walker.numbers(#binding, #name, #label, #dim); };
                        Some((walk, Named::Names(vec![name])))
                    }
                    "axes" => {
                        let Some(Expr::Path(kind)) = args.get(2) else {
                            return Err(syn::Error::new_spanned(list, "axes(name, label, kind)"));
                        };
                        let kind = kind
                            .path
                            .get_ident()
                            .map(ToString::to_string)
                            .unwrap_or_default();
                        let dim = dim(&kind);
                        let walk = quote! { walker.axes(#binding, #name, #label, #dim); };
                        let axes = ["x", "y", "z"].map(|a| format!("{name}{a}")).to_vec();
                        Some((walk, Named::Names(axes)))
                    }
                    "steps" => Some((
                        quote! { walker.steps(#binding, #name, #label); },
                        Named::Items,
                    )),
                    _ => return Err(syn::Error::new_spanned(&list.path, "an unknown role")),
                }
            }
            _ => return Err(syn::Error::new_spanned(attr, "an unknown role")),
        });
    }
    Ok(found.unwrap_or_else(|| {
        Some((
            quote! { ::core_document::walk::Walk::walk(#binding, walker); },
            Named::Nested,
        ))
    }))
}

fn dim(kind: &str) -> Tokens {
    match kind {
        "length" => quote!(Some(::core_document::expr::Dim::LENGTH)),
        "angle" => quote!(Some(::core_document::expr::Dim::ANGLE)),
        "number" => quote!(Some(::core_document::expr::Dim::NUMBER)),
        _ => quote!(None),
    }
}

/// `#[walk(setting)]` on the type itself.
fn container_is_setting(attrs: &[Attribute]) -> syn::Result<bool> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident("walk")) else {
        return Ok(false);
    };
    let meta: Meta = attr.parse_args()?;
    if !matches!(&meta, Meta::Path(p) if p.is_ident("setting")) {
        return Err(syn::Error::new_spanned(
            attr,
            "a type's only role is #[walk(setting)]",
        ));
    }
    Ok(true)
}

/// Whether one of the `#[serde(...)]` attributes names `word`.
fn serde_has(attrs: &[Attribute], word: &str) -> syn::Result<bool> {
    let mut found = false;
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(word) {
                found = true;
            }
            // Pass over whatever the item carries (`= "..."`, `(...)`).
            if meta.input.peek(Token![=]) {
                meta.value()?.parse::<Expr>()?;
            } else if meta.input.peek(syn::token::Paren) {
                meta.parse_nested_meta(|_| Ok(()))?;
            }
            Ok(())
        })?;
    }
    Ok(found)
}

/// The name `#[serde(rename = "...")]` gives, if any.
fn serde_rename(attrs: &[Attribute]) -> syn::Result<Option<String>> {
    let mut found = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                found = Some(lit.value());
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
