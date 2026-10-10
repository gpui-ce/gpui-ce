use crate::derive_reflect::schema_path;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Attribute, Ident, Path, Token, Visibility, parse::Parser, parse_quote, punctuated::Punctuated,
};

pub fn trait_set(input: TokenStream) -> TokenStream {
    let parser = Punctuated::<Path, Token![,]>::parse_terminated;

    match parser.parse(input).and_then(expand_trait_set) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_trait_set(traits: Punctuated<Path, Token![,]>) -> syn::Result<TokenStream2> {
    if traits.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "trait_set requires at least one trait",
        ));
    }

    let capabilities = format_ident!("__GpuiReflectionCapabilities", span = Span::mixed_site());
    let group = format_ident!("__GpuiReflectionGroup", span = Span::mixed_site());
    let capabilities_path = parse_quote!(#capabilities);
    let group_items = capability_group(&group, &Visibility::Inherited, &capabilities_path, &[]);
    let schemas = traits.iter().map(schema_path);

    let root_checks = traits.iter().enumerate().map(|(idx, root)| {
        let name = format_ident!(
            "__GPUI_REFLECTION_{group}_ROOT_CHECK_{idx}",
            span = Span::mixed_site(),
        );
        let schema = schema_path(root);

        quote! {
            #[allow(non_upper_case_globals)]
            const #name: #schema::Marker = #root;
        }
    });
    let roots = traits.iter();

    Ok(quote! {{
        trait #capabilities: #(#schemas::Capabilities)+* {}

        #group_items

        #(#root_checks)*

        ::gpui::reflection::ReflectedTraitGroup::<#group>::with_requirements(
            [#(::gpui::reflection::ReflectionToken::requirements(#roots)),*]
                .into_iter().flatten(),
        )
    }})
}

// Trait objects implement their supertraits, including inherited membership proofs.
pub(crate) fn capability_group(
    group: &Ident,
    visibility: &Visibility,
    capabilities: &Path,
    configurations: &[Attribute],
) -> TokenStream2 {
    let token = format_ident!("__GpuiReflectionToken", span = Span::mixed_site());

    quote! {
        #(#configurations)*
        #[doc(hidden)]
        #visibility struct #group;

        #(#configurations)*
        impl ::gpui::reflection::ReflectionGroup for #group {}

        #(#configurations)*
        impl<#token> ::gpui::reflection::IncludesReflectedTrait<#token> for #group
        where
            #token: ::gpui::reflection::ReflectionToken,
            dyn #capabilities: ::gpui::reflection::IncludesReflectedTrait<#token>,
        {
        }

        #(#configurations)*
        impl<#token> ::gpui::reflection::IncludesCallableTrait<#token> for #group
        where
            #token: ::gpui::reflection::CallableReflectionToken,
            dyn #capabilities: ::gpui::reflection::IncludesCallableTrait<#token>,
        {
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_trait_sets() {
        let error = expand_trait_set(Punctuated::new()).unwrap_err();

        assert_eq!(error.to_string(), "trait_set requires at least one trait");
    }
}
