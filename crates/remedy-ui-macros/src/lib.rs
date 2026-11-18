//! Derive macros for RemedyUI.
//!
//! `#[derive(Store)]` on a struct with named fields generates a trait, `<Name>Fields<R>`,
//! with one accessor per field. It is implemented for `Field<R, Name>` (a path into a store)
//! and for `Store<Name>` itself:
//!
//! ```
//! use remedy_ui_macros::Store;
//! use remedy_ui_reactive::Store;
//!
//! #[derive(Store, Clone, PartialEq)]
//! struct Message {
//!     body: String,
//!     edited: bool,
//! }
//!
//! let msg = Store::new(Message { body: "hi".into(), edited: false });
//! msg.edited().set(true)?;            // wakes readers of `edited`, not of `body`
//! assert_eq!(msg.body().get()?, "hi");
//! # Ok::<(), remedy_ui_reactive::StoreError>(())
//! ```
//!
//! The generated code names the reactive runtime the way the deriving crate depends on it:
//! through the facade (`remedy_ui::reactive`, with `remedy_ui::Store` as the derive) when it
//! depends on `remedy-ui`, or `remedy_ui_reactive` when it depends on that crate directly
//! (renamed dependencies are followed).
//!
//! Field names that would clash with `Field`'s own methods (`with`, `get`, `set`, `update`,
//! `set_if_changed`, `project`, `path`, `at`, `len`, `is_empty`) are rejected with a message,
//! because the accessor could never be called.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, parse_macro_input};

const RESERVED: &[&str] =
    &["with", "get", "set", "update", "set_if_changed", "project", "path", "at", "len", "is_empty"];

/// Generates field accessors for a reactive store. See the crate documentation.
#[proc_macro_derive(Store)]
pub fn derive_store(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The path generated code uses for the reactive runtime: the facade's `reactive` module when
/// the deriving crate depends on `remedy-ui`, else `remedy-ui-reactive` itself.
fn runtime_path() -> proc_macro2::TokenStream {
    let ident = |n: &str| proc_macro2::Ident::new(&n.replace('-', "_"), proc_macro2::Span::call_site());
    match crate_name("remedy-ui-reactive") {
        Ok(FoundCrate::Itself) => return quote!(crate),
        Ok(FoundCrate::Name(n)) => {
            let n = ident(&n);
            return quote!(::#n);
        }
        Err(_) => {}
    }
    match crate_name("remedy-ui") {
        Ok(FoundCrate::Name(n)) => {
            let n = ident(&n);
            quote!(::#n::reactive)
        }
        Ok(FoundCrate::Itself) => quote!(crate::reactive),
        // Neither is a dependency: the error names the crate to add.
        Err(_) => quote!(::remedy_ui_reactive),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let rt = runtime_path();
    let name = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[derive(Store)] does not support generic structs yet; wrap the generic value in a concrete struct",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "#[derive(Store)] works on structs with named fields"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "#[derive(Store)] needs named fields (not a tuple or unit struct)",
        ));
    };
    let vis = &input.vis;
    let trait_name = format_ident!("{}Fields", name);
    let mut sigs = Vec::new();
    let mut impls = Vec::new();
    let mut root_impls = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let Some(ident) = &field.ident else { continue };
        if RESERVED.contains(&ident.to_string().as_str()) {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "a field named `{ident}` clashes with Field::{ident}; rename it to use #[derive(Store)]"
                ),
            ));
        }
        let index = u32::try_from(index)
            .map_err(|_| syn::Error::new_spanned(ident, "too many fields for #[derive(Store)]"))?;
        let ty = &field.ty;
        let doc = format!("The `{ident}` field of `{name}`.");
        sigs.push(quote! {
            #[doc = #doc]
            fn #ident(&self) -> #rt::Field<R, #ty>;
        });
        impls.push(quote! {
            fn #ident(&self) -> #rt::Field<R, #ty> {
                self.project(#index, |v: &#name| &v.#ident, |v: &mut #name| &mut v.#ident)
            }
        });
        root_impls.push(quote! {
            fn #ident(&self) -> #rt::Field<#name, #ty> {
                <#rt::Field<#name, #name> as #trait_name<#name>>::#ident(&self.root())
            }
        });
    }
    let trait_doc = format!("Reactive field accessors for `{name}`, generated by `#[derive(Store)]`.");
    Ok(quote! {
        #[doc = #trait_doc]
        #vis trait #trait_name<R: 'static> {
            #(#sigs)*
        }

        impl<R: 'static> #trait_name<R> for #rt::Field<R, #name> {
            #(#impls)*
        }

        impl #trait_name<#name> for #rt::Store<#name> {
            #(#root_impls)*
        }
    })
}
