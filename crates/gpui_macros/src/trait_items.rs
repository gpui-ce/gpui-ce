use quote::quote;
use syn::{Attribute, ItemTrait, Meta, Token, TraitItem, parse_quote, punctuated::Punctuated};

pub(crate) enum UnknownMacros {
    Reject,
    #[cfg(any(feature = "inspector", debug_assertions))]
    Preserve,
}

pub(crate) fn normalize_trait_items(
    input: &mut ItemTrait,
    unknown_macros: UnknownMacros,
) -> syn::Result<()> {
    let mut normalized = Vec::new();

    for item in std::mem::take(&mut input.items) {
        let TraitItem::Macro(invocation) = item else {
            normalized.push(item);

            continue;
        };

        let path = &invocation.mac.path;
        let name = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        let generator = match name.as_str() {
            "gpui_macros::style_helpers" | "style_helpers" => crate::styles::style_helpers,
            "gpui_macros::visibility_style_methods" | "visibility_style_methods" => {
                crate::styles::visibility_style_methods
            }
            "gpui_macros::margin_style_methods" | "margin_style_methods" => {
                crate::styles::margin_style_methods
            }
            "gpui_macros::padding_style_methods" | "padding_style_methods" => {
                crate::styles::padding_style_methods
            }
            "gpui_macros::position_style_methods" | "position_style_methods" => {
                crate::styles::position_style_methods
            }
            "gpui_macros::overflow_style_methods" | "overflow_style_methods" => {
                crate::styles::overflow_style_methods
            }
            "gpui_macros::cursor_style_methods" | "cursor_style_methods" => {
                crate::styles::cursor_style_methods
            }
            "gpui_macros::border_style_methods" | "border_style_methods" => {
                crate::styles::border_style_methods
            }
            "gpui_macros::box_shadow_style_methods" | "box_shadow_style_methods" => {
                crate::styles::box_shadow_style_methods
            }
            _name if !matches!(unknown_macros, UnknownMacros::Reject) => {
                normalized.push(TraitItem::Macro(invocation));

                continue;
            }
            _name => {
                return Err(syn::Error::new_spanned(
                    path,
                    "reflect_trait cannot classify this trait-item macro; write its items directly, or have the macro generate the entire #[reflect_trait] trait",
                ));
            }
        };

        let output = generator(invocation.mac.tokens)?;
        let expanded: ItemTrait = syn::parse2(quote! { trait Generated { #output } })?;

        for mut item in expanded.items {
            let TraitItem::Fn(method) = &mut item else {
                return Err(syn::Error::new_spanned(
                    item,
                    "GPUI style macros must generate trait methods",
                ));
            };

            method.attrs.splice(0..0, invocation.attrs.iter().cloned());
            normalized.push(item);
        }
    }

    input.items = normalized;

    Ok(())
}

pub(crate) fn configuration_attributes(attributes: &[Attribute]) -> syn::Result<Vec<Attribute>> {
    attributes
        .iter()
        .filter_map(|attribute| {
            configuration_meta(&attribute.meta)
                .transpose()
                .map(|result| result.map(|meta| parse_quote!(#[#meta])))
        })
        .collect()
}

fn configuration_meta(meta: &Meta) -> syn::Result<Option<Meta>> {
    if meta.path().is_ident("cfg") {
        let Meta::List(list) = meta else {
            return Err(syn::Error::new_spanned(meta, "expected cfg(predicate)"));
        };

        list.parse_args::<Meta>()?;

        return Ok(Some(meta.clone()));
    }

    if !meta.path().is_ident("cfg_attr") {
        return Ok(None);
    }

    let Meta::List(list) = meta else {
        return Err(syn::Error::new_spanned(
            meta,
            "expected cfg_attr(predicate, attributes...)",
        ));
    };

    let arguments = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;

    if arguments.len() < 2 {
        return Err(syn::Error::new_spanned(
            meta,
            "cfg_attr requires a predicate and at least one attribute",
        ));
    }

    let mut arguments = arguments.iter();
    let condition = arguments.next().unwrap();
    let configurations = arguments
        .filter_map(|argument| configuration_meta(argument).transpose())
        .collect::<syn::Result<Vec<_>>>()?;

    if configurations.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        parse_quote!(cfg_attr(#condition, #(#configurations),*)),
    ))
}
