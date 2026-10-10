use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Member, Meta, parse_macro_input};

fn delegates(attribute: &Attribute) -> syn::Result<bool> {
    match &attribute.meta {
        Meta::Path(_path) => Ok(false),
        Meta::List(_list) => {
            let setting = attribute.parse_args::<syn::Ident>()?;

            if setting != "delegate" {
                return Err(syn::Error::new_spanned(setting, "expected delegate"));
            }

            Ok(true)
        }
        Meta::NameValue(_value) => Err(syn::Error::new_spanned(
            attribute,
            "expected a field marker or delegate",
        )),
    }
}

fn marked_field(input: &DeriveInput, attribute_name: &str) -> syn::Result<(Member, bool)> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "element trait derives require a struct",
        ));
    };

    let mut found = None;

    for (idx, field) in data.fields.iter().enumerate() {
        for attribute in &field.attrs {
            if !attribute.path().is_ident(attribute_name) {
                continue;
            }

            let delegate = delegates(attribute)?;
            let member = match &data.fields {
                Fields::Named(_fields) => Member::Named(field.ident.clone().expect("named field")),
                Fields::Unnamed(_fields) => Member::Unnamed(idx.into()),
                Fields::Unit => unreachable!(),
            };

            if found.replace((member, delegate)).is_some() {
                return Err(syn::Error::new_spanned(
                    attribute,
                    format!("only one #[{attribute_name}] field is allowed"),
                ));
            }
        }
    }

    found.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            format!("derive requires a #[{attribute_name}] field"),
        )
    })
}

fn expand_accessor(
    input: &DeriveInput,
    attribute: &str,
    trait_path: TokenStream2,
    method: TokenStream2,
    return_type: TokenStream2,
) -> syn::Result<TokenStream2> {
    let (field, delegate) = marked_field(input, attribute)?;
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let accessor = if delegate {
        quote! { #trait_path::#method(&mut self.#field) }
    } else {
        quote! { &mut self.#field }
    };

    Ok(quote! {
        impl #impl_generics #trait_path for #name #type_generics #where_clause {
            fn #method(&mut self) -> &mut #return_type {
                #accessor
            }
        }
    })
}

pub fn derive_styled(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_accessor(
        &input,
        "style",
        quote!(gpui::Styled),
        quote!(style),
        quote!(gpui::StyleRefinement),
    )
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

pub fn derive_interactive_element(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_accessor(
        &input,
        "interactivity",
        quote!(gpui::InteractiveElement),
        quote!(interactivity),
        quote!(gpui::Interactivity),
    )
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

pub fn derive_parent_element(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_parent_element(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_parent_element(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let (field, delegate) = marked_field(input, "children")?;
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let extend = if delegate {
        quote! { gpui::ParentElement::extend(&mut self.#field, elements); }
    } else {
        quote! { self.#field.extend(elements); }
    };

    Ok(quote! {
        impl #impl_generics gpui::ParentElement for #name #type_generics #where_clause {
            fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
                #extend
            }
        }
    })
}

pub fn derive_stateful_interactive_element(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics gpui::StatefulInteractiveElement for #name #type_generics #where_clause {}
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn rejects_missing_duplicate_and_invalid_field_markers() {
        for (declaration, expected) in [
            ("enum Missing { Item }", "require a struct"),
            ("struct Missing;", "requires a #[style] field"),
            (
                "struct Duplicate { #[style] first: (), #[style] second: () }",
                "only one",
            ),
            (
                "struct Duplicate { #[style] #[style(delegate)] field: () }",
                "only one",
            ),
            (
                "struct Invalid { #[style(unknown)] field: () }",
                "expected delegate",
            ),
            (
                "struct Invalid { #[style = true] field: () }",
                "expected a field marker",
            ),
        ] {
            let input = syn::parse_str(declaration).unwrap();
            let error = marked_field(&input, "style").unwrap_err();

            assert!(error.to_string().contains(expected), "{error}");
        }

        let input = parse_quote! { struct Invalid { #[children(unknown)] field: () } };

        assert!(expand_parent_element(&input).is_err());
    }
}
