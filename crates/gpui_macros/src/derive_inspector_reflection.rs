//! Generates inspector metadata for parameterless owned builders returning `Self`.

use crate::trait_items::{UnknownMacros, configuration_attributes, normalize_trait_items};
use heck::ToSnakeCase as _;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    Attribute, Expr, FnArg, Ident, Item, ItemTrait, Lit, Meta, ReturnType, TraitItem, Type,
    parse_macro_input,
};

pub fn derive_inspector_reflection(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as Item);
    let Item::Trait(mut trait_item) = item else {
        return syn::Error::new_spanned(
            item,
            "#[derive_inspector_reflection] can only be applied to traits",
        )
        .to_compile_error()
        .into();
    };

    normalize_trait_items(&mut trait_item, UnknownMacros::Preserve)
        .and_then(|()| generate_reflected_trait(trait_item))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

fn generate_reflected_trait(trait_item: ItemTrait) -> syn::Result<TokenStream2> {
    let trait_name = &trait_item.ident;
    let vis = &trait_item.vis;

    let call_site = Span::call_site();
    let inspector_reflection_path = if is_called_from_gpui_crate(call_site) {
        quote! { crate::inspector_reflection }
    } else {
        quote! { ::gpui::inspector_reflection }
    };

    let mut method_infos = Vec::new();

    for item in &trait_item.items {
        if let TraitItem::Fn(method) = item {
            let method_name = &method.sig.ident;

            let has_valid_self_receiver = method
                .sig
                .inputs
                .iter()
                .any(|arg| matches!(arg, FnArg::Receiver(r) if r.reference.is_none()));

            let returns_self = match &method.sig.output {
                ReturnType::Type(_, ty) => {
                    matches!(**ty, Type::Path(ref path) if path.path.is_ident("Self"))
                }
                ReturnType::Default => false,
            };

            let param_count = method.sig.inputs.len();

            if has_valid_self_receiver && returns_self && param_count == 1 {
                let doc = extract_doc_comment(&method.attrs);
                let cfg_attrs = configuration_attributes(&method.attrs)?;
                method_infos.push((method_name.clone(), doc, cfg_attrs));
            }
        }
    }

    let reflection_mod_name = Ident::new(
        &format!("{}_reflection", trait_name.to_string().to_snake_case()),
        trait_name.span(),
    );

    let wrapper_functions = method_infos.iter().map(|(method_name, _doc, cfg_attrs)| {
        let wrapper_name = Ident::new(
            &format!("__wrapper_{}", method_name),
            method_name.span(),
        );
        quote! {
            #(#cfg_attrs)*
            fn #wrapper_name<T: #trait_name + 'static>(value: Box<dyn std::any::Any>) -> Box<dyn std::any::Any> {
                if let Ok(concrete) = value.downcast::<T>() {
                    Box::new(concrete.#method_name())
                } else {
                    panic!("Type mismatch in reflection wrapper");
                }
            }
        }
    });

    let method_info_entries = method_infos.iter().map(|(method_name, doc, cfg_attrs)| {
        let method_name_str = method_name.to_string();
        let wrapper_name = Ident::new(&format!("__wrapper_{}", method_name), method_name.span());
        let doc_expr = match doc {
            Some(doc_str) => quote! { Some(#doc_str) },
            None => quote! { None },
        };
        quote! {
            #(#cfg_attrs)*
            #inspector_reflection_path::FunctionReflection {
                name: #method_name_str,
                function: #wrapper_name::<T>,
                documentation: #doc_expr,
                _type: ::std::marker::PhantomData,
            }
        }
    });

    let output = quote! {
        #trait_item

        /// Inspector metadata for owned builder methods.
        #vis mod #reflection_mod_name {
            use super::*;

            #(#wrapper_functions)*

            /// Returns the reflected builders for this concrete type.
            pub fn methods<T: #trait_name + 'static>() -> Vec<#inspector_reflection_path::FunctionReflection<T>> {
                vec![
                    #(#method_info_entries),*
                ]
            }

            /// Finds a reflected builder by name.
            pub fn find_method<T: #trait_name + 'static>(name: &str) -> Option<#inspector_reflection_path::FunctionReflection<T>> {
                methods::<T>().into_iter().find(|m| m.name == name)
            }
        }
    };

    Ok(output)
}

fn extract_doc_comment(attrs: &[Attribute]) -> Option<String> {
    let mut doc_lines = Vec::new();

    for attr in attrs {
        if attr.path().is_ident("doc")
            && let Meta::NameValue(meta) = &attr.meta
            && let Expr::Lit(expr_lit) = &meta.value
            && let Lit::Str(lit_str) = &expr_lit.lit
        {
            let line = lit_str.value();
            let line = line.strip_prefix(' ').unwrap_or(&line);
            doc_lines.push(line.to_string());
        }
    }

    if doc_lines.is_empty() {
        None
    } else {
        Some(doc_lines.join("\n"))
    }
}

fn is_called_from_gpui_crate(_span: Span) -> bool {
    // Use the invoking package name to choose internal or external imports.
    std::env::var("CARGO_PKG_NAME").is_ok_and(|name| name == "gpui")
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn reflects_configured_builder_metadata() {
        let mut input: ItemTrait = parse_quote! {
            trait Styled {
                fn style(&mut self);

                /// Configured visibility.
                #[cfg_attr(all(), cfg_attr(all(), cfg(any()), allow(dead_code)))]
                visibility_style_methods!();
            }
        };
        normalize_trait_items(&mut input, UnknownMacros::Preserve).unwrap();
        let output = generate_reflected_trait(input).unwrap().to_string();

        let configuration = quote! {
            #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
        }
        .to_string();

        assert_eq!(output.matches(&configuration).count(), 4);
        assert!(output.contains("Configured visibility."));
        assert!(output.contains("name : \"visible\""));
        assert!(output.contains("name : \"invisible\""));
    }
}
