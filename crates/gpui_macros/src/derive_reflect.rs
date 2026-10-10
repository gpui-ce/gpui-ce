use crate::{
    trait_items::{UnknownMacros, configuration_attributes, normalize_trait_items},
    trait_set::capability_group,
};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    DeriveInput, FnArg, GenericArgument, GenericParam, ItemTrait, Lifetime, LifetimeParam, Meta,
    Path, PathArguments, ReturnType, Signature, Token, TraitBoundModifier, TraitItem, TraitItemFn,
    Type, TypeParamBound, parse_macro_input, parse_quote,
    punctuated::Punctuated,
    visit_mut::{self, VisitMut},
};

pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_reflect(input) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_reflect(input: DeriveInput) -> syn::Result<TokenStream2> {
    let type_name = &input.ident;
    let generic = !input.generics.params.is_empty();
    let automatic_traits: [Path; 4] = [
        parse_quote!(gpui::Styled),
        parse_quote!(gpui::InteractiveElement),
        parse_quote!(gpui::StatefulInteractiveElement),
        parse_quote!(gpui::ParentElement),
    ];
    let mut explicit_traits = Vec::new();
    let mut explicit_list = false;

    for attribute in &input.attrs {
        if attribute.path().is_ident("reflect") {
            explicit_list = true;
            explicit_traits.extend(
                attribute.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)?,
            );
        }
    }

    if generic && !explicit_list {
        return Err(syn::Error::new_spanned(
            type_name,
            "generic Reflect derives require an explicit #[reflect(...)] list, including builtin traits; use #[reflect()] for an empty registration",
        ));
    }

    let mut generics = input.generics.clone();

    if generic {
        let bounds = generics.make_where_clause();
        bounds.predicates.push(parse_quote!(Self: 'static));
        bounds.predicates.extend(
            explicit_traits
                .iter()
                .map(|path| -> syn::WherePredicate { parse_quote!(Self: #path) }),
        );
    }

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let probes = automatic_traits.iter().filter(|_path| !generic).map(|trait_path| {
        quote! {
            {
                struct Probe<Type>(::std::marker::PhantomData<Type>);

                trait Detect {
                    fn register(self, implementations: &mut Vec<gpui::reflection::ReflectedImplementation>);
                }

                impl<Type> Detect for &Probe<Type> {
                    fn register(self, _implementations: &mut Vec<gpui::reflection::ReflectedImplementation>) {}
                }

                impl<Type: #trait_path + 'static> Detect for &&Probe<Type> {
                    fn register(self, implementations: &mut Vec<gpui::reflection::ReflectedImplementation>) {
                        #trait_path.__register::<Type>(implementations);
                    }
                }

                (&&Probe::<Self>(::std::marker::PhantomData)).register(&mut implementations);
            }
        }
    });
    let explicit = explicit_traits.iter().map(|trait_path| {
        quote! { #trait_path.__register::<Self>(&mut implementations); }
    });
    let registration = (!generic).then(|| {
        quote! {
            gpui::private::inventory::submit! {
                gpui::reflection::ReflectionRegistration {
                    type_id: || ::std::any::TypeId::of::<#type_name>(),
                    reflection: <#type_name as gpui::reflection::Reflect>::reflection,
                }
            }
        }
    });

    Ok(quote! {
        impl #impl_generics gpui::reflection::Reflect for #type_name #type_generics #where_clause {
            fn build_reflection() -> Vec<gpui::reflection::ReflectedImplementation> {
                let mut implementations = Vec::new();
                #(#probes)*
                #(#explicit)*

                implementations
            }
        }

        #registration
    })
}

pub fn reflect_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    let membership = args.to_string() == "membership";
    let input = parse_macro_input!(input as ItemTrait);
    let expanded = if args.is_empty() {
        expand_trait(input)
    } else if membership {
        expand_membership_trait(input)
    } else {
        Err(syn::Error::new(
            Span::call_site(),
            "expected #[reflect_trait] or #[reflect_trait(membership)]",
        ))
    };

    match expanded {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_trait(mut input: ItemTrait) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "reflect_trait requires a non-generic trait without a where clause",
        ));
    }

    if input.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "unsafe traits cannot be forwarded through reflected elements",
        ));
    }

    normalize_trait_items(&mut input, UnknownMacros::Reject)?;

    let name = &input.ident;
    let visibility = &input.vis;
    let marker = format_ident!("__GpuiReflect{name}");
    let group = format_ident!("__GpuiReflect{name}Group");
    let methods = format_ident!("__GpuiReflect{name}Methods");
    let schema = schema_path(&parse_quote!(#name));
    let schema_name = &schema.segments.last().unwrap().ident;
    let mut parents = Vec::new();

    for bound in &input.supertraits {
        let TypeParamBound::Trait(bound) = bound else {
            return Err(syn::Error::new_spanned(
                bound,
                "reflected supertraits must be non-generic trait paths",
            ));
        };

        if bound.path.is_ident("Sized") {
            continue;
        }

        if bound.modifier != TraitBoundModifier::None
            || bound.lifetimes.is_some()
            || bound
                .path
                .segments
                .iter()
                .any(|segment| !matches!(segment.arguments, PathArguments::None))
        {
            return Err(syn::Error::new_spanned(
                bound,
                "reflected supertraits must be non-generic trait paths",
            ));
        }

        parents.push(bound.path.clone());
    }

    let parent_aliases = (0..parents.len())
        .map(|idx| format_ident!("Parent{idx}"))
        .collect::<Vec<_>>();
    let parent_imports = (0..parents.len())
        .map(|idx| format_ident!("__GpuiReflect{name}Parent{idx}"))
        .collect::<Vec<_>>();
    let parent_schemas = parents.iter().map(schema_path);
    let mut forwarded = Vec::new();

    for item in &mut input.items {
        match item {
            TraitItem::Fn(method) => {
                if let MethodDispatch::Concrete(signature) = classify_method(method)? {
                    forwarded.push(forward_method(method, name, signature)?);
                }
            }
            TraitItem::Type(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "reflected traits cannot have associated types",
                ));
            }
            TraitItem::Const(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "callable reflected traits cannot expose associated constants; move a shared value outside the trait, or expose a borrowed method for concrete values",
                ));
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "reflect_trait cannot classify this trait item; use a supported method declaration",
                ));
            }
        }
    }

    let fields = forwarded.iter().map(|method| &method.field);
    let initializers = forwarded.iter().map(|method| &method.initializer);
    let implementations = forwarded.iter().map(|method| &method.implementation);
    let parent_members = parents.iter().zip(&parent_aliases).map(|(parent, alias)| {
        quote! {
            __GpuiReflectionGroup: gpui::reflection::IncludesCallableTrait<#schema_name::#alias::Marker>,
            gpui::reflection::ReflectedElement<__GpuiReflectionGroup>: #parent,
        }
    });
    let configurations = configuration_attributes(&input.attrs)?;
    let parent_import_items = parent_schemas.zip(&parent_imports).map(|(schema, import)| {
        quote! {
            #(#configurations)*
            #[doc(hidden)]
            pub use #schema as #import;
        }
    });
    let parent_checks =
        parents
            .iter()
            .zip(&parent_aliases)
            .enumerate()
            .map(|(idx, (parent, alias))| {
                let check = format_ident!("__GPUI_REFLECTION_PARENT_CHECK_{name}_{idx}");

                quote! {
                    #(#configurations)*
                    #[allow(non_upper_case_globals)]
                    const #check: #schema_name::#alias::Marker = #parent;
                }
            });
    let group_items = capability_group(
        &group,
        &parse_quote!(pub),
        &parse_quote!(#schema_name::Capabilities),
        &configurations,
    );

    let requirements = (!parents.is_empty()).then(|| {
        quote! {
            fn requirements(self) -> Vec<gpui::reflection::ReflectionRequirement> {
                let mut requirements = vec![gpui::reflection::ReflectionToken::requirement(self)];

                #(for requirement in gpui::reflection::ReflectionToken::requirements(
                    #parents,
                ) {
                    if !requirements.contains(&requirement) {
                        requirements.push(requirement);
                    }
                })*

                requirements
            }
        }
    });
    let supertraits = (!parents.is_empty()).then(|| {
        quote! {
            .with_supertraits(|| {
                static PARENTS: ::std::sync::LazyLock<Vec<gpui::reflection::ReflectedTrait>>
                    = ::std::sync::LazyLock::new(|| vec![
                        #(gpui::reflection::ReflectionToken::reflected_trait(
                            #parents,
                        )),*
                    ]);

                &PARENTS
            })
        }
    });

    let register = quote! {
        gpui::reflection::__register_callable(
            self,
            implementations,
            || #methods { #(#initializers)* },
        )
    };
    let registration = if parents.is_empty() {
        quote! { #register; }
    } else {
        quote! {
            if !#register {
                return;
            }

            #(#parents.__register::<Type>(implementations);)*
        }
    };

    Ok(quote! {
        #input

        #(#configurations)*
        #[doc(hidden)]
        #[derive(Clone, Copy, Default)]
        pub struct #marker;

        #(#configurations)*
        #[doc(hidden)]
        pub struct #methods { #(#fields)* }

        #(#parent_import_items)*
        #(#parent_checks)*

        #(#configurations)*
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub mod #schema_name {
            pub use self::Capabilities as CallableCapabilities;
            #(pub use super::#parent_imports as #parent_aliases;)*

            pub type Marker = super::#marker;

            pub trait Capabilities:
                gpui::reflection::IncludesCallableTrait<Marker>
                #(+ #parent_aliases::CallableCapabilities)*
            {}
        }

        #group_items

        #(#configurations)*
        impl gpui::reflection::ReflectionToken for #marker {
            type Group = #group;

            fn requires_adapter(self) -> bool {
                true
            }

            fn requirement(self) -> gpui::reflection::ReflectionRequirement {
                gpui::reflection::ReflectionRequirement::callable(self)
            }

            #requirements

            fn reflected_trait(self) -> gpui::reflection::ReflectedTrait {
                gpui::reflection::ReflectedTrait::new(
                    concat!(module_path!(), "::", stringify!(#name)),
                    || ::std::any::TypeId::of::<#marker>(),
                ) #supertraits
            }
        }

        #(#configurations)*
        impl gpui::reflection::CallableReflectionToken for #marker {
            type Methods = #methods;
        }

        #(#configurations)*
        impl #marker {
            #[doc(hidden)]
            #[allow(private_bounds)]
            pub fn __register<Type: #name + 'static>(
                self,
                implementations: &mut Vec<gpui::reflection::ReflectedImplementation>,
            ) {
                #registration
            }
        }

        #(#configurations)*
        impl<__GpuiReflectionGroup> #name for gpui::reflection::ReflectedElement<__GpuiReflectionGroup>
        where
            __GpuiReflectionGroup: gpui::reflection::IncludesCallableTrait<#marker>,
            #(#parent_members)*
        {
            #(#implementations)*
        }

        #(#configurations)*
        #[doc = concat!("Descriptor for the ", stringify!(#name), " trait.")]
        #[allow(non_upper_case_globals)]
        #visibility const #name: #marker = #marker;
    })
}

fn expand_membership_trait(input: ItemTrait) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "membership reflection requires a non-generic trait without a where clause",
        ));
    }

    let name = &input.ident;
    let visibility = &input.vis;
    let marker = format_ident!("__GpuiReflect{name}");
    let schema = schema_path(&parse_quote!(#name));
    let schema_name = &schema.segments.last().unwrap().ident;
    let configurations = configuration_attributes(&input.attrs)?;

    Ok(quote! {
        #input

        #(#configurations)*
        #[doc(hidden)]
        #[derive(Clone, Copy, Default)]
        pub struct #marker;

        #(#configurations)*
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub mod #schema_name {
            pub type Marker = super::#marker;

            pub trait Capabilities: gpui::reflection::IncludesReflectedTrait<Marker> {}
        }

        #(#configurations)*
        impl gpui::reflection::ReflectionToken for #marker {
            type Group = gpui::reflection::ErasedReflectionGroup;

            fn reflected_trait(self) -> gpui::reflection::ReflectedTrait {
                gpui::reflection::ReflectedTrait::new(
                    concat!(module_path!(), "::", stringify!(#name)),
                    || ::std::any::TypeId::of::<#marker>(),
                )
            }

            fn requires_adapter(self) -> bool {
                false
            }
        }

        #(#configurations)*
        impl #marker {
            #[doc(hidden)]
            #[allow(private_bounds)]
            pub fn __register<Type: #name + 'static>(
                self,
                implementations: &mut Vec<gpui::reflection::ReflectedImplementation>,
            ) {
                implementations.push(gpui::reflection::ReflectedImplementation {
                    descriptor: gpui::reflection::ReflectionToken::reflected_trait(self),
                    methods: None,
                });
            }
        }

        #(#configurations)*
        #[doc = concat!("Membership descriptor for the ", stringify!(#name), " trait.")]
        #[allow(non_upper_case_globals)]
        #visibility const #name: #marker = #marker;
    })
}

pub(crate) fn schema_path(path: &Path) -> Path {
    let mut schema = path.clone();
    let segment = schema.segments.last_mut().unwrap();
    segment.ident = format_ident!("__GpuiReflect{}Schema", segment.ident);

    schema
}

enum MethodDispatch {
    Concrete(ForwardedSignature),
    WrapperDefault,
}

fn classify_method(method: &mut TraitItemFn) -> syn::Result<MethodDispatch> {
    let mut wrapper_default = false;
    let mut attributes = Vec::new();

    for attribute in std::mem::take(&mut method.attrs) {
        if !attribute.path().is_ident("reflect") {
            reject_conditional_wrapper_default(&attribute.meta)?;
            attributes.push(attribute);

            continue;
        }

        let settings =
            attribute.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;

        if settings.is_empty() {
            return Err(syn::Error::new_spanned(
                attribute,
                "expected #[reflect(wrapper_default)] on a provided method",
            ));
        }

        for setting in settings {
            if !matches!(&setting, Meta::Path(path) if path.is_ident("wrapper_default")) {
                return Err(syn::Error::new_spanned(
                    setting,
                    "unknown reflected method setting; expected wrapper_default",
                ));
            }

            if wrapper_default {
                return Err(syn::Error::new_spanned(
                    setting,
                    "duplicate wrapper_default setting",
                ));
            }

            wrapper_default = true;
        }
    }

    method.attrs = attributes;
    configuration_attributes(&method.attrs)?;

    if wrapper_default {
        if method.default.is_none() {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "wrapper_default requires a provided method body",
            ));
        }

        return Ok(MethodDispatch::WrapperDefault);
    }

    if method.default.is_some()
        && matches!(method.sig.inputs.first(), Some(FnArg::Receiver(receiver))
            if receiver.reference.is_none() && receiver.colon_token.is_none())
    {
        return Ok(MethodDispatch::WrapperDefault);
    }

    let signature = validate_forwarded_signature(&method.sig).map_err(|mut error| {
        if method.default.is_some() {
            error.combine(syn::Error::new_spanned(
                &method.sig.ident,
                "use #[reflect(wrapper_default)] to intentionally run this provided body on ReflectedElement; concrete overrides will not be dispatched",
            ));
        }

        error
    })?;

    Ok(MethodDispatch::Concrete(signature))
}

fn reject_conditional_wrapper_default(meta: &Meta) -> syn::Result<()> {
    if !meta.path().is_ident("cfg_attr") {
        return Ok(());
    }

    let Meta::List(list) = meta else {
        return Err(syn::Error::new_spanned(meta, "expected cfg_attr arguments"));
    };
    let arguments = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;

    for attribute in arguments.iter().skip(1) {
        if attribute.path().is_ident("reflect") {
            return Err(syn::Error::new_spanned(
                attribute,
                "conditional reflected method settings are unsupported; use #[reflect(wrapper_default)] directly",
            ));
        }

        reject_conditional_wrapper_default(attribute)?;
    }

    Ok(())
}

struct ForwardedSignature {
    signature: Signature,
    lifetimes: Punctuated<GenericParam, Token![,]>,
    element_lifetime: Lifetime,
    mutable: bool,
    erased_arguments: Vec<TokenStream2>,
    argument_names: Vec<syn::Ident>,
    preparations: Vec<TokenStream2>,
    output: ReturnType,
}

struct ForwardedMethod {
    field: TokenStream2,
    initializer: TokenStream2,
    implementation: TokenStream2,
}

fn validate_forwarded_signature(signature: &Signature) -> syn::Result<ForwardedSignature> {
    if signature.asyncness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.constness.is_some()
        || signature.variadic.is_some()
        || signature.generics.where_clause.is_some()
        || signature.generics.params.iter().any(|parameter| {
            !matches!(parameter, GenericParam::Lifetime(parameter) if parameter.bounds.is_empty())
        })
    {
        return Err(syn::Error::new_spanned(
            signature,
            "forwarded reflected methods support borrowed receivers, lifetime parameters, and concrete types",
        ));
    }

    let Some(FnArg::Receiver(receiver)) = signature.inputs.first() else {
        return Err(syn::Error::new_spanned(
            signature,
            "forwarded reflected methods must have an &self or &mut self receiver",
        ));
    };

    if receiver.colon_token.is_some() {
        return Err(syn::Error::new_spanned(
            receiver,
            "typed receivers are not supported",
        ));
    }

    let Some((_, lifetime)) = &receiver.reference else {
        return Err(syn::Error::new_spanned(
            receiver,
            "forwarded reflected methods must have an &self or &mut self receiver",
        ));
    };

    let lifetime = lifetime.as_ref().filter(|lifetime| lifetime.ident != "_");
    let element_lifetime = lifetime
        .cloned()
        .unwrap_or_else(|| Lifetime::new("'__gpui_element", Span::mixed_site()));
    let mut lifetimes = signature.generics.params.clone();

    if lifetime.is_none() {
        lifetimes.push(GenericParam::Lifetime(LifetimeParam::new(
            element_lifetime.clone(),
        )));
    }

    let mut signature = signature.clone();
    let mut erased_arguments = Vec::new();
    let mut argument_names = Vec::new();
    let mut preparations = Vec::new();

    for (idx, argument) in signature.inputs.iter_mut().skip(1).enumerate() {
        let FnArg::Typed(argument) = argument else {
            unreachable!()
        };
        let argument_name = format_ident!("argument_{idx}");
        *argument.pat = parse_quote!(#argument_name);
        argument_names.push(argument_name.clone());

        if let Type::ImplTrait(iterator) = argument.ty.as_ref() {
            let bounds = iterator.bounds.iter().collect::<Vec<_>>();
            let [TypeParamBound::Trait(bound)] = bounds.as_slice() else {
                return Err(syn::Error::new_spanned(
                    iterator,
                    "only impl IntoIterator<Item = ConcreteType> can be erased",
                ));
            };
            let segment = bound.path.segments.last().unwrap();
            let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return Err(syn::Error::new_spanned(
                    iterator,
                    "an iterator Item type is required",
                ));
            };
            let arguments = arguments.args.iter().collect::<Vec<_>>();
            let [GenericArgument::AssocType(item)] = arguments.as_slice() else {
                return Err(syn::Error::new_spanned(
                    iterator,
                    "an iterator Item type is required",
                ));
            };

            if segment.ident != "IntoIterator"
                || item.ident != "Item"
                || bound.modifier != TraitBoundModifier::None
                || bound.lifetimes.is_some()
            {
                return Err(syn::Error::new_spanned(
                    iterator,
                    "only impl IntoIterator<Item = ConcreteType> can be erased",
                ));
            }

            validate_type(&item.ty)?;
            let item_type = &item.ty;
            erased_arguments.push(quote! { &mut dyn ::std::iter::Iterator<Item = #item_type> });
            preparations.push(quote! { let mut #argument_name = #argument_name.into_iter(); });

            continue;
        }

        validate_type(&argument.ty)?;
        let argument_type = &argument.ty;
        erased_arguments.push(quote! { #argument_type });
    }

    let mut output = signature.output.clone();

    if let ReturnType::Type(_, output_type) = &mut output {
        validate_type(output_type)?;
        OutputLifetimes(&element_lifetime).visit_type_mut(output_type);
    }

    Ok(ForwardedSignature {
        signature,
        lifetimes,
        element_lifetime,
        mutable: receiver.mutability.is_some(),
        erased_arguments,
        argument_names,
        preparations,
        output,
    })
}

fn forward_method(
    method: &TraitItemFn,
    trait_name: &syn::Ident,
    lowered: ForwardedSignature,
) -> syn::Result<ForwardedMethod> {
    let ForwardedSignature {
        signature,
        lifetimes,
        element_lifetime,
        mutable,
        erased_arguments,
        argument_names,
        preparations,
        output,
    } = lowered;

    let method_name = &signature.ident;

    let erased_receiver = if mutable {
        quote! { &#element_lifetime mut dyn ::std::any::Any }
    } else {
        quote! { &#element_lifetime dyn ::std::any::Any }
    };

    let downcast = if mutable {
        quote! { downcast_mut }
    } else {
        quote! { downcast_ref }
    };

    let parts = if mutable {
        quote! { __reflection_parts_mut }
    } else {
        quote! { __reflection_parts }
    };

    let forwarded_arguments = signature.inputs.iter().skip(1).zip(&argument_names).map(|(argument, name)| {
        if matches!(argument, FnArg::Typed(argument) if matches!(*argument.ty, Type::ImplTrait(_))) {
            return quote! { &mut #name };
        }

        quote! { #name }
    });
    let configurations = configuration_attributes(&method.attrs)?;
    let lifetime_binder = if lifetimes.is_empty() {
        quote! {}
    } else {
        quote! { for<#lifetimes> }
    };

    Ok(ForwardedMethod {
        field: quote! {
            #(#configurations)*
            #method_name: #lifetime_binder fn(#erased_receiver, #(#erased_arguments),*) #output,
        },
        initializer: quote! {
            #(#configurations)*
            #method_name: |element, #(#argument_names),*| {
                let concrete = element.#downcast::<Type>().expect("reflected element type changed");

                <Type as #trait_name>::#method_name(concrete, #(#argument_names),*)
            },
        },
        implementation: quote! {
            #(#configurations)*
            #signature {
                #(#preparations)*
                let (methods, element) = self.#parts(#trait_name);

                (methods.#method_name)(element, #(#forwarded_arguments),*)
            }
        },
    })
}

fn validate_type(type_name: &Type) -> syn::Result<()> {
    struct UnsupportedType(Option<syn::Error>);

    impl VisitMut for UnsupportedType {
        fn visit_type_mut(&mut self, type_name: &mut Type) {
            if matches!(type_name, Type::ImplTrait(_))
                || matches!(type_name, Type::Path(path) if path.path.segments.iter().any(|segment| segment.ident == "Self"))
            {
                self.0 = Some(syn::Error::new_spanned(
                    type_name,
                    "forwarded reflected methods cannot use Self or impl Trait outside a supported iterator argument",
                ));

                return;
            }

            visit_mut::visit_type_mut(self, type_name);
        }
    }

    let mut visitor = UnsupportedType(None);
    visitor.visit_type_mut(&mut type_name.clone());

    match visitor.0 {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct OutputLifetimes<'life>(&'life Lifetime);

impl VisitMut for OutputLifetimes<'_> {
    fn visit_type_bare_fn_mut(&mut self, _function: &mut syn::TypeBareFn) {}

    fn visit_parenthesized_generic_arguments_mut(
        &mut self,
        _arguments: &mut syn::ParenthesizedGenericArguments,
    ) {
    }

    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident == "_")
        {
            reference.lifetime = Some(self.0.clone());
        }

        visit_mut::visit_type_reference_mut(self, reference);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_explicit_generic_registration_and_membership_declarations() {
        for declaration in [
            "#[reflect(gpui::Styled)] struct Panel<State>(State) where State: Clone;",
            "#[reflect()] struct Buffer<const COUNT: usize>([u8; COUNT]);",
            "#[reflect(Marker)] struct Borrowed<'data>(&'data str);",
        ] {
            let output = expand_reflect(syn::parse_str(declaration).unwrap()).unwrap();

            syn::parse2::<syn::File>(output).unwrap();
        }

        let error = expand_reflect(parse_quote! { struct Missing<State>(State); }).unwrap_err();

        assert!(error.to_string().contains("explicit #[reflect(...)] list"));

        let output = expand_membership_trait(parse_quote! {
            trait PaintSource: std::fmt::Debug + Send {
                type Brush;
                const PALETTE_SIZE: usize;
                fn paint<Input>(&self, input: Input) -> Self::Brush;
            }
        })
        .unwrap();

        syn::parse2::<syn::File>(output).unwrap();

        for declaration in [
            "trait Generic<State> {}",
            "trait Conditional where Self: Send {}",
        ] {
            assert!(expand_membership_trait(syn::parse_str(declaration).unwrap()).is_err());
        }
    }

    fn assert_rejected(item: &str, expected: &str) {
        let input = syn::parse_str(&format!("trait Invalid {{ {item} }}")).unwrap();
        let error = expand_trait(input).unwrap_err().to_compile_error();

        assert!(error.to_string().contains(expected), "{item}: {error}");
    }

    #[test]
    fn rejects_signatures_that_cannot_be_forwarded() {
        for declaration in [
            "trait Generic<Item> {}",
            "trait Associated { type Item; }",
            "trait Owned { fn consume(self); }",
            "trait ReturningSelf { fn copy(&self) -> Self; }",
            "trait GenericMethod { fn update<Type>(&mut self, value: Type); }",
            "trait AsyncMethod { async fn update(&mut self); }",
        ] {
            assert!(expand_trait(syn::parse_str(declaration).unwrap()).is_err());
        }
    }

    #[test]
    fn accepts_borrowed_defaults_and_wrapper_methods() {
        for method in [
            "fn read(&self) -> &str { \"default\" }",
            "fn build<Input>(self, value: Input) -> Self { self }",
            "#[reflect(wrapper_default)] fn edit<Input>(&mut self, value: Input) {}",
            "#[reflect(wrapper_default)] fn read(&self) -> usize { 12 }",
            "#[reflect(wrapper_default)] fn consume(self: Box<Self>) {}",
        ] {
            let input = syn::parse_str(&format!("trait Supported: Sized {{ {method} }}")).unwrap();
            let output = expand_trait(input).unwrap();

            syn::parse2::<syn::File>(output).unwrap();
        }
    }

    #[test]
    fn rejects_unsupported_defaults_and_invalid_wrapper_settings() {
        for (item, message) in [
            (
                "fn generic<Input>(&self, value: Input) {}",
                "wrapper_default",
            ),
            (
                "fn copy(&self) -> Self { unreachable!() }",
                "cannot use Self",
            ),
            ("fn consume(self: Box<Self>) {}", "typed receivers"),
            ("fn create() -> usize { 0 }", "&self or &mut self"),
            (
                "async fn update(&mut self) {}",
                "forwarded reflected methods",
            ),
            (
                "fn update(&mut self) where Self: Sized {}",
                "forwarded reflected methods",
            ),
            (
                "#[reflect(wrapper_default)] fn required(&self);",
                "requires a provided method body",
            ),
            (
                "#[reflect(unknown)] fn read(&self) {}",
                "unknown reflected method setting",
            ),
            (
                "#[reflect(wrapper_default = true)] fn read(&self) {}",
                "unknown reflected method setting",
            ),
            (
                "#[reflect()] fn read(&self) {}",
                "expected #[reflect(wrapper_default)]",
            ),
            (
                "#[reflect(wrapper_default, wrapper_default)] fn read(&self) {}",
                "duplicate wrapper_default",
            ),
            (
                "#[reflect(wrapper_default)] #[reflect(wrapper_default)] fn read(&self) {}",
                "duplicate wrapper_default",
            ),
            (
                "#[cfg_attr(all(), reflect(wrapper_default))] fn read(&self) {}",
                "conditional reflected method settings",
            ),
        ] {
            assert_rejected(item, message);
        }
    }

    #[test]
    fn rejects_constants_macros_and_unclassified_items() {
        for item in ["const LIMIT: usize;", "const LIMIT: usize = 12;"] {
            assert_rejected(item, "cannot expose associated constants");
        }

        assert_rejected(
            "#[cfg(any())] external_methods!();",
            "cannot classify this trait-item macro",
        );

        let mut input: ItemTrait = parse_quote! { trait Invalid {} };
        input
            .items
            .push(TraitItem::Verbatim(quote! { unsupported; }));

        assert!(
            expand_trait(input)
                .unwrap_err()
                .to_string()
                .contains("cannot classify this trait item")
        );
    }

    #[test]
    fn retains_nested_configuration() {
        let input: TraitItemFn = parse_quote! {
            #[cfg(feature = "enabled")]
            #[cfg_attr(all(), cfg_attr(all(), cfg(any()), allow(dead_code)), doc = "disabled")]
            fn content(&self);
        };

        let expected: TraitItemFn = parse_quote! {
            #[cfg(feature = "enabled")]
            #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
            fn content(&self);
        };

        assert_eq!(
            configuration_attributes(&input.attrs).unwrap(),
            expected.attrs
        );
    }

    #[test]
    fn rejects_malformed_configuration() {
        for attribute in [
            "#[cfg]",
            "#[cfg_attr(all())]",
            "#[cfg_attr(all(), cfg_attr(all()))]",
        ] {
            assert_rejected(&format!("{attribute} fn read(&self) {{}}"), "cfg");
        }
    }

    #[test]
    fn preserves_order_and_attributes_when_normalizing_macros() {
        let mut input: ItemTrait = parse_quote! {
            trait Styled {
                fn before(&self);

                /// Generated styles.
                #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
                visibility_style_methods!();

                fn after(&self);
            }
        };
        let TraitItem::Macro(invocation) = &input.items[1] else {
            unreachable!();
        };

        let attributes = invocation.attrs.clone();
        normalize_trait_items(&mut input, UnknownMacros::Reject).unwrap();

        for (idx, name) in ["before", "visible", "invisible", "after"]
            .iter()
            .enumerate()
        {
            let TraitItem::Fn(method) = &input.items[idx] else {
                panic!("expected a normalized method");
            };

            assert_eq!(method.sig.ident, name);

            if (1..3).contains(&idx) {
                assert!(method.attrs.starts_with(&attributes));
            }
        }

        expand_trait(input).unwrap();
        assert_rejected("style_helpers!(invalid);", "unexpected token");
    }
}
