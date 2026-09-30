use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use std::collections::HashSet;
use syn::{Path, Token, parse::Parser, punctuated::Punctuated};

pub fn trait_set(input: TokenStream) -> TokenStream {
    let parser = Punctuated::<Path, Token![,]>::parse_terminated;

    let traits = match parser.parse(input) {
        Ok(traits) => traits,
        Err(error) => return error.to_compile_error().into(),
    };

    if traits.is_empty() {
        return syn::Error::new(Span::call_site(), "trait_set requires at least one trait")
            .to_compile_error()
            .into();
    }

    let mut marker_paths = Vec::new();
    let mut seen_markers = HashSet::new();

    for reflected_trait in &traits {
        push_marker_path(reflected_trait, &mut marker_paths, &mut seen_markers);

        if reflected_trait
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "StatefulInteractiveElement")
        {
            let mut interactive = reflected_trait.clone();
            interactive.segments.last_mut().unwrap().ident = format_ident!("InteractiveElement");
            push_marker_path(&interactive, &mut marker_paths, &mut seen_markers);
        }
    }

    let group = format_ident!("__GpuiReflectionGroup", span = Span::mixed_site());
    let trait_paths = traits.iter().cloned().collect::<Vec<_>>();

    quote! {{
        #[doc(hidden)]
        struct #group;

        impl ::gpui::reflection::ReflectionGroup for #group {}

        #(
            impl ::gpui::reflection::IncludesReflectedTrait<#marker_paths> for #group {}
        )*

        ::gpui::reflection::ReflectedTraitGroup::<#group>::new([
            #(
                ::gpui::reflection::ReflectionToken::reflected_trait(#trait_paths)
            ),*
        ])
    }}
    .into()
}

fn push_marker_path(
    reflected_trait: &Path,
    marker_paths: &mut Vec<Path>,
    seen_markers: &mut HashSet<String>,
) {
    let mut marker = reflected_trait.clone();
    let name = marker.segments.last().unwrap().ident.clone();
    marker.segments.last_mut().unwrap().ident = format_ident!("__GpuiReflect{}", name);
    let marker_key = marker.to_token_stream().to_string();

    if seen_markers.insert(marker_key) {
        marker_paths.push(marker);
    }
}
