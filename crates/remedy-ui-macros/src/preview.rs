//! `#[preview]` and `#[previews]`: previews declared next to the UI they show.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Expr, ExprLit, ItemFn, ItemMod, Lit, LitStr, Path};

/// What a `#[preview(...)]` attribute says.
pub(crate) struct Args {
    name: Option<LitStr>,
    themes: Vec<LitStr>,
    locales: Vec<LitStr>,
    sizes: Vec<(f64, f64)>,
    text_scales: Vec<f64>,
    krate: Option<Path>,
    test: bool,
    golden: Option<LitStr>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            name: None,
            themes: Vec::new(),
            locales: Vec::new(),
            sizes: Vec::new(),
            text_scales: Vec::new(),
            krate: None,
            test: true,
            golden: None,
        }
    }
}

fn number(e: &Expr) -> syn::Result<f64> {
    match e {
        Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) => i.base10_parse::<f64>(),
        Expr::Lit(ExprLit { lit: Lit::Float(f), .. }) => f.base10_parse::<f64>(),
        Expr::Group(g) => number(&g.expr),
        Expr::Paren(p) => number(&p.expr),
        other => Err(syn::Error::new(other.span(), "expected a number")),
    }
}

fn string(e: &Expr) -> syn::Result<LitStr> {
    match e {
        Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => Ok(s.clone()),
        Expr::Group(g) => string(&g.expr),
        other => Err(syn::Error::new(other.span(), "expected a string")),
    }
}

fn size(e: &Expr) -> syn::Result<(f64, f64)> {
    match e {
        Expr::Tuple(t) if t.elems.len() == 2 => {
            let mut it = t.elems.iter();
            match (it.next(), it.next()) {
                (Some(w), Some(h)) => Ok((number(w)?, number(h)?)),
                _ => Err(syn::Error::new(t.span(), "a size is (width, height)")),
            }
        }
        Expr::Group(g) => size(&g.expr),
        other => Err(syn::Error::new(other.span(), "a size is (width, height)")),
    }
}

fn list<T>(e: &Expr, one: impl Fn(&Expr) -> syn::Result<T>) -> syn::Result<Vec<T>> {
    match e {
        Expr::Array(a) => a.elems.iter().map(one).collect(),
        Expr::Group(g) => list(&g.expr, one),
        other => Ok(vec![one(other)?]),
    }
}

impl Args {
    pub(crate) fn parse(attr: TokenStream) -> syn::Result<Args> {
        let mut args = Args::default();
        let parser = syn::meta::parser(|meta| {
            let key = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
            if key == "crate" {
                args.krate = Some(meta.value()?.parse::<Path>()?);
                return Ok(());
            }
            let value: Expr = meta.value()?.parse()?;
            match key.as_str() {
                "name" => args.name = Some(string(&value)?),
                "theme" | "themes" => args.themes = list(&value, string)?,
                "locale" | "locales" => args.locales = list(&value, string)?,
                "size" | "sizes" => args.sizes = list(&value, size)?,
                "text_scale" | "text_scales" => args.text_scales = list(&value, number)?,
                "golden" => args.golden = Some(string(&value)?),
                "test" => match &value {
                    Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) => args.test = b.value,
                    other => return Err(syn::Error::new(other.span(), "test = true or false")),
                },
                _ => {
                    return Err(meta.error(
                        "unknown preview setting; use theme(s), locale(s), size(s), text_scale(s), name, golden, test or crate",
                    ));
                }
            }
            Ok(())
        });
        syn::parse::Parser::parse2(parser, attr)?;
        Ok(args)
    }
}

/// The name of the constant `#[preview]` makes for the function `f`.
fn spec_name(f: &syn::Ident) -> syn::Ident {
    format_ident!("{}", f.to_string().to_uppercase(), span = f.span())
}

pub(crate) fn expand_preview(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(attr)?;
    let f: ItemFn = syn::parse2(item)?;
    if f.sig.inputs.len() != 1 {
        return Err(syn::Error::new(
            f.sig.span(),
            "a preview is `fn name(ui: &Ui) -> El`: it builds its UI into the preview's view",
        ));
    }
    let fname = &f.sig.ident;
    let vis = &f.vis;
    let krate = args.krate.clone().unwrap_or_else(|| syn::parse_quote!(::remedy_ui::devtools));
    let name = args.name.clone().unwrap_or_else(|| LitStr::new(&fname.to_string(), fname.span()));
    let themes = &args.themes;
    let locales = &args.locales;
    let sizes = args.sizes.iter().map(|(w, h)| {
        let (w, h) = (*w as f32, *h as f32);
        quote!((#w, #h))
    });
    let scales = args.text_scales.iter().map(|s| *s as f32);
    let spec = spec_name(fname);
    let doc = format!("The preview `#[preview]` declared on [`{fname}`]: its variations and builder.");
    let test = if args.test {
        let test_name = format_ident!("{}_matches_its_golden_images", fname);
        let golden = args.golden.clone().unwrap_or_else(|| LitStr::new("tests/golden", Span::call_site()));
        quote! {
            /// Each variation of the preview, rendered by the CPU renderer, matches its golden
            /// image (the first run writes the images for review).
            #[cfg(test)]
            #[test]
            fn #test_name() {
                let dir = ::std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(#golden);
                let result = #krate::golden_test(&#spec, &dir);
                assert!(result.is_ok(), "{}", result.err().map(|e| e.to_string()).unwrap_or_default());
            }
        }
    } else {
        TokenStream::new()
    };
    Ok(quote! {
        #f

        #[doc = #doc]
        #[allow(dead_code, reason = "read by the preview gallery and the golden test, which may be the only users")]
        #vis const #spec: #krate::PreviewSpec = #krate::PreviewSpec {
            name: #name,
            themes: &[#(#themes),*],
            locales: &[#(#locales),*],
            sizes: &[#(#sizes),*],
            text_scales: &[#(#scales),*],
            build: #fname,
        };

        #test
    })
}

fn is_preview(a: &syn::Attribute) -> bool {
    a.path().segments.last().is_some_and(|s| s.ident == "preview")
}

pub(crate) fn expand_previews(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let mut krate: Option<Path> = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("crate") {
            krate = Some(meta.value()?.parse::<Path>()?);
            Ok(())
        } else {
            Err(meta.error("#[previews] takes only `crate = path`"))
        }
    });
    syn::parse::Parser::parse2(parser, attr)?;
    let mut m: ItemMod = syn::parse2(item)?;
    let Some((_, items)) = &mut m.content else {
        return Err(syn::Error::new(
            m.span(),
            "#[previews] goes on a module with its previews inside: `mod previews { ... }`",
        ));
    };
    let mut specs = Vec::new();
    for it in items.iter() {
        if let syn::Item::Fn(f) = it
            && f.attrs.iter().any(is_preview)
        {
            specs.push(spec_name(&f.sig.ident));
        }
    }
    let krate = krate.unwrap_or_else(|| syn::parse_quote!(::remedy_ui::devtools));
    items.push(syn::parse_quote! {
        /// Every preview declared in this module, for the preview gallery.
        pub fn all() -> ::std::vec::Vec<#krate::Preview> {
            ::std::vec![#(#krate::PreviewSpec::preview(&#specs)),*]
        }
    });
    Ok(quote!(#m))
}
