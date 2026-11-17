//! `#[derive(Snapshot)]`: what a world image may hold of a type, from the
//! type's definition (romacs-snapshot's `SnapshotSafe` and `Schema`).
//!
//! The derive sees every field and every variant, so none can be left out:
//! each is walked (`Walk::field`, which requires it SnapshotSafe) and
//! settled, but those declared `#[snapshot(at_rest)]`.
//!
//! - `#[snapshot(at_rest)]` on a variant: its payload is the type's to keep
//!   inert in an image (romacs-vm's `Object::foreign_resource` refuses a live
//!   one), and is neither walked nor settled.
//! - `#[snapshot(root)]` on a struct: the image's root, which enters an image
//!   only through a door that validates it (romacs-vm's
//!   `world::image::capture`), so it is not made SnapshotSafe. Only a root
//!   may have at-rest fields (`#[snapshot(at_rest)]` on a field): the
//!   restoring process remakes them as their defaults (`Schema::renew`).
//!
//! A type parameter is required SnapshotSafe and Schema itself.
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Error, Fields, Index, Member, parse_macro_input};

#[proc_macro_derive(Snapshot, attributes(snapshot))]
pub fn derive_snapshot(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input).unwrap_or_else(Error::into_compile_error).into()
}

/// What `#[snapshot(...)]` says of an item.
#[derive(Default)]
struct Marks {
    root: bool,
    at_rest: bool,
}

fn marks(attrs: &[Attribute]) -> syn::Result<Marks> {
    let mut marks = Marks::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("snapshot")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("root") {
                marks.root = true;
            } else if meta.path.is_ident("at_rest") {
                marks.at_rest = true;
            } else {
                return Err(meta.error("expected `root` or `at_rest`"));
            }
            Ok(())
        })?;
    }
    Ok(marks)
}

/// The `#[cfg(...)]` attributes of an item, which its generated code keeps.
fn cfgs(attrs: &[Attribute]) -> Vec<&Attribute> {
    attrs.iter().filter(|attr| attr.path().is_ident("cfg")).collect()
}

/// One field: how the code names it, the label a census shows, and whether
/// it is at rest.
struct Field<'a> {
    member: Member,
    label: String,
    cfgs: Vec<&'a Attribute>,
    at_rest: bool,
}

fn fields(fields: &Fields) -> syn::Result<Vec<Field<'_>>> {
    fields.iter().enumerate().map(|(at, field)| {
        let (member, label) = if let Some(name) = &field.ident {
            (Member::Named(name.clone()), name.to_string())
        } else {
            (Member::Unnamed(Index::from(at)), at.to_string())
        };
        Ok(Field { member, label, cfgs: cfgs(&field.attrs), at_rest: marks(&field.attrs)?.at_rest })
    }).collect()
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let root = marks(&input.attrs)?.root;
    if let Some(lifetime) = input.generics.lifetimes().next() {
        return Err(Error::new_spanned(lifetime, "an image holds no borrow: a Snapshot type has no lifetime"));
    }
    let mut generics = input.generics.clone();
    for parameter in generics.type_params_mut() {
        parameter.bounds.push(syn::parse_quote!(::romacs_snapshot::SnapshotSafe));
        parameter.bounds.push(syn::parse_quote!(::romacs_snapshot::Schema));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let (walk, settle, renew) = match &input.data {
        Data::Struct(data) => {
            let fields = fields(&data.fields)?;
            if !root && let Some(field) = fields.iter().find(|field| field.at_rest) {
                return Err(Error::new_spanned(&field.member, "only a `#[snapshot(root)]` has at-rest fields: no restore remakes one anywhere else"));
            }
            struct_bodies(&fields)
        }
        Data::Enum(data) => {
            if root {
                return Err(Error::new(Span::call_site(), "a root is a struct"));
            }
            enum_bodies(data)?
        }
        Data::Union(data) => return Err(Error::new_spanned(data.union_token, "a union's fields cannot be walked")),
    };
    let safe = (!root).then(|| quote! {
        // SAFETY: derived from the definition: every field is walked and
        // required SnapshotSafe (`Walk::field`), but declared at-rest
        // variants, which the type keeps inert in an image.
        unsafe impl #impl_generics ::romacs_snapshot::SnapshotSafe for #name #type_generics #where_clause {}
    });
    // Only a root has fields to remake; every other type keeps the default.
    let renew = root.then(|| quote! { fn renew(&mut self) { #renew } });
    Ok(quote! {
        #safe
        impl #impl_generics ::romacs_snapshot::Schema for #name #type_generics #where_clause {
            fn walk(&self, w: &mut ::romacs_snapshot::Walk<'_>) { let _ = &w; #walk }
            fn any(&self) -> ::std::option::Option<&dyn ::std::any::Any> { ::std::option::Option::Some(self) }
            fn settle(&mut self, s: &mut ::romacs_snapshot::Settle) { let _ = &s; #settle }
            #renew
        }
    })
}

/// A struct's walk, settling and renewal, field by field.
fn struct_bodies(fields: &[Field<'_>]) -> (Tokens, Tokens, Tokens) {
    let (walked, rest): (Vec<_>, Vec<_>) = fields.iter().partition(|field| !field.at_rest);
    let walk = walked.iter().map(|Field { member, label, cfgs, .. }| quote! { #(#cfgs)* w.field(#label, &self.#member); });
    let settle = walked.iter().map(|Field { member, cfgs, .. }| quote! { #(#cfgs)* ::romacs_snapshot::Schema::settle(&mut self.#member, s); });
    let renew = rest.iter().map(|Field { member, cfgs, .. }| quote! { #(#cfgs)* { self.#member = ::std::default::Default::default(); } });
    (quote! { #(#walk)* }, quote! { #(#settle)* }, quote! { #(#renew)* })
}

/// An enum's walk and settling, variant by variant: a variant is named to
/// the walk, then its payload's fields are met, unless it is at rest.
fn enum_bodies(data: &syn::DataEnum) -> syn::Result<(Tokens, Tokens, Tokens)> {
    let mut walk = Vec::new();
    let mut settle = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let label = ident.to_string();
        let cfgs = cfgs(&variant.attrs);
        let fields = fields(&variant.fields)?;
        if let Some(field) = fields.iter().find(|field| field.at_rest) {
            return Err(Error::new_spanned(&field.member, "a variant's payload is at rest whole: mark the variant"));
        }
        if marks(&variant.attrs)?.at_rest || fields.is_empty() {
            let pattern = match &variant.fields {
                Fields::Named(_) => quote! { Self::#ident { .. } },
                Fields::Unnamed(_) => quote! { Self::#ident(..) },
                Fields::Unit => quote! { Self::#ident },
            };
            walk.push(quote! { #(#cfgs)* #pattern => w.variant(#label), });
            settle.push(quote! { #(#cfgs)* #pattern => {} });
            continue;
        }
        // Bound to names of the derive's own, never a field's, which could be
        // the walk's or the settling's argument.
        let bindings: Vec<_> = (0..fields.len()).map(|at| format_ident!("f{}", at)).collect();
        let members = fields.iter().map(|field| &field.member);
        let pattern = if let Fields::Named(_) = &variant.fields {
            quote! { Self::#ident { #(#members: #bindings),* } }
        } else {
            quote! { Self::#ident(#(#bindings),*) }
        };
        let labels = fields.iter().map(|field| &field.label);
        walk.push(quote! { #(#cfgs)* #pattern => { w.variant(#label); #(w.field(#labels, #bindings);)* } });
        settle.push(quote! { #(#cfgs)* #pattern => { #(::romacs_snapshot::Schema::settle(#bindings, s);)* } });
    }
    let empty = data.variants.is_empty();
    let (walk, settle) = if empty {
        (quote! { match *self {} }, quote! { match *self {} })
    } else {
        (quote! { match self { #(#walk)* } }, quote! { match self { #(#settle)* } })
    };
    Ok((walk, settle, Tokens::new()))
}
