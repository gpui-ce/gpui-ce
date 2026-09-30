use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    DeriveInput, ItemTrait, Path, Token, TraitItem, parse_macro_input, parse_quote,
    punctuated::Punctuated,
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
        let trait_name = &trait_path.segments.last().unwrap().ident;

        quote! {
            {
                struct Probe<Type>(::std::marker::PhantomData<Type>);
                trait Detect {
                    fn reflected(self) -> Option<gpui::reflection::ReflectedTrait>;
                }
                impl<Type> Detect for &Probe<Type> {
                    fn reflected(self) -> Option<gpui::reflection::ReflectedTrait> {
                        None
                    }
                }
                impl<Type: #trait_path> Detect for &&Probe<Type> {
                    fn reflected(self) -> Option<gpui::reflection::ReflectedTrait> {
                        Some(gpui::reflection::ReflectionToken::reflected_trait(
                            gpui::#trait_name,
                        ))
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
                let reflected_trait = gpui::reflection::ReflectionToken::reflected_trait(#trait_path);

                if !traits.contains(&reflected_trait) {
                    traits.push(reflected_trait);
                }
            }
        }
    });

    let reflected_capabilities = quote! {
        || {
            let style = {
                struct Probe<Type>(::std::marker::PhantomData<Type>);
                trait Detect {
                    fn reflected_style(
                        self,
                    ) -> Option<gpui::reflection::ReflectedStyleAccessor>;
                }
                impl<Type> Detect for &Probe<Type> {
                    fn reflected_style(
                        self,
                    ) -> Option<gpui::reflection::ReflectedStyleAccessor> {
                        None
                    }
                }
                impl<Type: gpui::Styled + 'static> Detect for &&Probe<Type> {
                    fn reflected_style(
                        self,
                    ) -> Option<gpui::reflection::ReflectedStyleAccessor> {
                        fn style<Type: gpui::Styled + 'static>(
                            element: &mut dyn ::std::any::Any,
                        ) -> &mut gpui::StyleRefinement {
                            <Type as gpui::Styled>::style(
                                element
                                    .downcast_mut::<Type>()
                                    .expect("reflected element type changed"),
                            )
                        }

                        Some(style::<Type>)
                    }
                }

                (&&Probe::<#type_name>(::std::marker::PhantomData)).reflected_style()
            };

            let interactivity = {
                struct Probe<Type>(::std::marker::PhantomData<Type>);
                trait Detect {
                    fn reflected_interactivity(
                        self,
                    ) -> Option<gpui::reflection::ReflectedInteractivityAccessor>;
                }
                impl<Type> Detect for &Probe<Type> {
                    fn reflected_interactivity(
                        self,
                    ) -> Option<gpui::reflection::ReflectedInteractivityAccessor> {
                        None
                    }
                }
                impl<Type: gpui::InteractiveElement + 'static> Detect for &&Probe<Type> {
                    fn reflected_interactivity(
                        self,
                    ) -> Option<gpui::reflection::ReflectedInteractivityAccessor> {
                        fn interactivity<Type: gpui::InteractiveElement + 'static>(
                            element: &mut dyn ::std::any::Any,
                        ) -> &mut gpui::Interactivity {
                            <Type as gpui::InteractiveElement>::interactivity(
                                element
                                    .downcast_mut::<Type>()
                                    .expect("reflected element type changed"),
                            )
                        }

                        Some(interactivity::<Type>)
                    }
                }

                (&&Probe::<#type_name>(::std::marker::PhantomData)).reflected_interactivity()
            };

            let extend = {
                struct Probe<Type>(::std::marker::PhantomData<Type>);
                trait Detect {
                    fn reflected_extend(
                        self,
                    ) -> Option<gpui::reflection::ReflectedParentExtender>;
                }
                impl<Type> Detect for &Probe<Type> {
                    fn reflected_extend(
                        self,
                    ) -> Option<gpui::reflection::ReflectedParentExtender> {
                        None
                    }
                }
                impl<Type: gpui::ParentElement + 'static> Detect for &&Probe<Type> {
                    fn reflected_extend(
                        self,
                    ) -> Option<gpui::reflection::ReflectedParentExtender> {
                        fn extend<Type: gpui::ParentElement + 'static>(
                            element: &mut dyn ::std::any::Any,
                            children: Vec<gpui::AnyElement>,
                        ) {
                            <Type as gpui::ParentElement>::extend(
                                element
                                    .downcast_mut::<Type>()
                                    .expect("reflected element type changed"),
                                children,
                            );
                        }

                        Some(extend::<Type>)
                    }
                }

                (&&Probe::<#type_name>(::std::marker::PhantomData)).reflected_extend()
            };

            gpui::reflection::ReflectedCapabilities {
                style,
                interactivity,
                extend,
            }
        }
    };

    quote! {
        impl gpui::reflection::Reflect for #type_name {
            fn reflected_traits() -> Vec<gpui::reflection::ReflectedTrait> {
                let mut traits = Vec::new();
                #(#preset_probes)*
                #(#custom_registrations)*
                traits
            }
        }

        gpui::private::inventory::submit! {
            gpui::reflection::ReflectionRegistration {
                type_id: || ::std::any::TypeId::of::<#type_name>(),
                traits: <#type_name as gpui::reflection::Reflect>::reflected_traits,
                capabilities: #reflected_capabilities,
            }
        }
    }
    .into()
}

pub fn reflect_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    let preset = if args.is_empty() {
        None
    } else {
        Some(parse_macro_input!(args as syn::Ident))
    };

    let input = parse_macro_input!(input as ItemTrait);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(&input.ident, "reflect_trait requires a non-generic trait")
            .to_compile_error()
            .into();
    }

    let name = &input.ident;
    let visibility = &input.vis;
    let marker = format_ident!("__GpuiReflect{}", name);
    let group = format_ident!("__GpuiReflect{}Group", name);
    let has_required_items = input.items.iter().any(|item| match item {
        TraitItem::Const(item) => item.default.is_none(),
        TraitItem::Fn(item) => item.default.is_none(),
        TraitItem::Type(item) => item.default.is_none(),
        TraitItem::Macro(_) | TraitItem::Verbatim(_) | _ => false,
    });

    if preset.is_none() && (!input.supertraits.is_empty() || has_required_items) {
        return syn::Error::new_spanned(
            name,
            "custom reflected traits must have no supertraits or required items",
        )
        .to_compile_error()
        .into();
    }

    let selected_element_impl =
        if preset.is_none() && input.supertraits.is_empty() && !has_required_items {
            quote! {
                impl<Group> #name for gpui::reflection::ReflectedElement<Group>
                where
                    Group: gpui::reflection::IncludesReflectedTrait<#marker>,
                {
                }
            }
        } else {
            quote! {}
        };
    let inherited_membership = match preset.as_ref().map(syn::Ident::to_string).as_deref() {
        Some("stateful_interactive") => {
            let interactive_marker = format_ident!("__GpuiReflectInteractiveElement");
            quote! {
                impl gpui::reflection::IncludesReflectedTrait<
                    gpui::#interactive_marker,
                > for #group {}
            }
        }
        Some("styled" | "interactive" | "parent") | None => quote! {},
        Some(preset) => {
            return syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("unknown reflect_trait preset `{preset}`"),
            )
            .to_compile_error()
            .into();
        }
    };

    let trait_name = if preset.is_some() {
        quote! { concat!("gpui::", stringify!(#name)) }
    } else {
        quote! { concat!(module_path!(), "::", stringify!(#name)) }
    };

    let reflected_items = quote! {
        #[doc(hidden)]
        #[derive(Clone, Copy)]
        #visibility struct #marker;

        #[doc(hidden)]
        #visibility struct #group;

        impl gpui::reflection::ReflectionGroup for #group {}

        impl gpui::reflection::IncludesReflectedTrait<#marker> for #group {}

        #inherited_membership

        impl gpui::reflection::ReflectionToken for #marker {
            type Group = #group;

            fn reflected_trait(self) -> gpui::reflection::ReflectedTrait {
                gpui::reflection::ReflectedTrait::new(
                    #trait_name,
                    || ::std::any::TypeId::of::<#marker>(),
                )
            }
        }

        #selected_element_impl

        #[doc = concat!("Descriptor for the `", stringify!(#name), "` trait.")]
        #[allow(non_upper_case_globals)]
        #visibility const #name: #marker = #marker;
    };

    quote! {
        #input

        #reflected_items
    }
    .into()
}
