use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

use crate::get_simple_attribute_field;

pub fn derive_app_context(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let Some(app_variable) = get_simple_attribute_field(&ast, "app") else {
        return quote! {
            compile_error!("Derive must have an #[app] attribute to detect the &mut App field");
        }
        .into();
    };

    let type_name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();

    let r#gen = quote! {
        impl #impl_generics gpui::context::AppContext for #type_name #type_generics
        #where_clause
        {}

        impl #impl_generics gpui::app_borrow::UpgradeApp for #type_name #type_generics
        #where_clause
        {
            fn upgrade(&self) -> impl gpui::app_borrow::BorrowApp {
                self
            }

            fn upgrade_mut(&mut self) -> impl gpui::app_borrow::BorrowAppMut {
                self
            }
        }
        impl #impl_generics gpui::app_borrow::BorrowApp for &#type_name #type_generics
        #where_clause
        {
            fn borrow(&self) -> impl std::ops::Deref<Target = gpui::App> {
                <#type_name as std::ops::Deref>::deref(self)
            }
        }
        impl #impl_generics gpui::app_borrow::BorrowAppMut for &mut #type_name #type_generics
        #where_clause
        {
            fn borrow_mut(&mut self) -> impl std::ops::DerefMut<Target = gpui::App> {
                <#type_name as std::ops::DerefMut>::deref_mut(self)
            }
        }
        impl #impl_generics std::ops::Deref for #type_name #type_generics
        #where_clause
        {
            type Target = gpui::App;
            fn deref(&self) -> &Self::Target {
                self.#app_variable
            }
        }
        impl #impl_generics std::ops::DerefMut for #type_name #type_generics
        #where_clause
        {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.#app_variable
            }
        }

        impl #impl_generics gpui::context::ImpliedAppEntity for #type_name #type_generics
        #where_clause
        {}
        impl #impl_generics gpui::context::AppContextBorrowEntity for #type_name #type_generics
        #where_clause
        {
            fn as_mut<'y, 'z, T>(
                &'y mut self,
                handle: &'z gpui::Entity<T>,
            ) -> gpui::GpuiBorrow<'y, T>
            where
                T: 'static,
            {
                self.#app_variable.as_mut(handle)
            }
        }
        impl #impl_generics gpui::context::ImpliedAppWindow for #type_name #type_generics
        #where_clause
        {}
        impl #impl_generics gpui::context::ImpliedAppTask for #type_name #type_generics
        #where_clause
        {}
        impl #impl_generics gpui::context::ImpliedAppGlobals for #type_name #type_generics
        #where_clause
        {}
    };

    r#gen.into()
}
