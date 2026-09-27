use proc_macro::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, ItemTrait, Path, Token, parse_macro_input, parse_quote, punctuated::Punctuated,
};

pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &input.ident,
            "Reflect currently requires a concrete, non-generic type",
        )
        .to_compile_error()
        .into();
    }

    let type_name = &input.ident;
    let preset_traits: [Path; 4] = [
        parse_quote!(gpui::Styled),
        parse_quote!(gpui::InteractiveElement),
        parse_quote!(gpui::StatefulInteractiveElement),
        parse_quote!(gpui::ParentElement),
    ];
    let mut custom_traits = Vec::new();

    for attribute in &input.attrs {
        if attribute.path().is_ident("reflect") {
            let traits = attribute.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated);

            match traits {
                Ok(traits) => custom_traits.extend(traits),
                Err(error) => {
                    return error.to_compile_error().into();
                }
            }
        }
    }

    let preset_probes = preset_traits.iter().map(|trait_path| {
        quote! {
            {
                struct Probe<Type>(::std::marker::PhantomData<Type>);
                trait Detect {
                    fn reflected(self) -> Option<gpui::ReflectedTrait>;
                }
                impl<Type> Detect for &Probe<Type> {
                    fn reflected(self) -> Option<gpui::ReflectedTrait> {
                        None
                    }
                }
                impl<Type: #trait_path> Detect for &&Probe<Type> {
                    fn reflected(self) -> Option<gpui::ReflectedTrait> {
                        Some(#trait_path)
                    }
                }

                if let Some(reflected_trait) =
                    (&&Probe::<Self>(::std::marker::PhantomData)).reflected()
                {
                    traits.push(reflected_trait);
                }
            }
        }
    });

    let custom_registrations = custom_traits.iter().map(|trait_path| {
        quote! {
            {
                fn check_trait<Type: #trait_path>() {}
                check_trait::<Self>();
                let reflected_trait = #trait_path;

                if !traits.contains(&reflected_trait) {
                    traits.push(reflected_trait);
                }
            }
        }
    });

    quote! {
        impl gpui::Reflect for #type_name {
            fn reflected_traits() -> Vec<gpui::ReflectedTrait> {
                let mut traits = Vec::new();
                #(#preset_probes)*
                #(#custom_registrations)*
                traits
            }
        }

        gpui::private::inventory::submit! {
            gpui::private::ReflectionRegistration {
                type_id: || ::std::any::TypeId::of::<#type_name>(),
                traits: <#type_name as gpui::Reflect>::reflected_traits,
            }
        }
    }
    .into()
}

pub fn reflect_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "reflect_trait takes no arguments",
        )
        .to_compile_error()
        .into();
    }

    let input = parse_macro_input!(input as ItemTrait);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(&input.ident, "reflect_trait requires a non-generic trait")
            .to_compile_error()
            .into();
    }

    let name = &input.ident;
    let visibility = &input.vis;

    quote! {
        #input

        #[doc = concat!("Descriptor for the `", stringify!(#name), "` trait.")]
        #[allow(non_upper_case_globals)]
        #visibility const #name: gpui::ReflectedTrait = gpui::ReflectedTrait::new(
            concat!(module_path!(), "::", stringify!(#name)),
            || {
                struct TraitMarker;
                ::std::any::TypeId::of::<TraitMarker>()
            },
        );
    }
    .into()
}
