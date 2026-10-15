//! Procedural macros for GPUI.
//!
//! `gpui` re-exports these macros, so use them as `gpui::test`,
//! `#[derive(gpui::Action)]`, and so on. Application crates do not need a
//! direct dependency on this crate.
//!
//! Derives:
//!
//! - `Action`: implements `gpui::Action` for a type that you can bind to keys.
//! - `IntoElement`, `Render`: implement the traits that let a type enter the element tree.
//! - `Styled`, `ParentElement`, `InteractiveElement`, `StatefulInteractiveElement`:
//!   implement the builder traits for a type that holds a field for them.
//! - `AppContext`, `VisualContext`: forward context methods from a wrapper around
//!   `&mut App` or `&mut Window`.
//!
//! Attributes and function-like macros:
//!
//! - `#[gpui::test]`, `#[gpui::property_test]` and `#[gpui::bench]` define
//!   tests and benchmarks that run with GPUI support.
//! - `register_action!` registers a hand-written `Action` impl.
//! - The `*_style_methods!` macros generate the builder methods of `Styled`.

mod bench;
mod derive_action;
mod derive_app_context;
mod derive_element_traits;
mod derive_into_element;
mod derive_render;
mod derive_visual_context;
mod property_test;
mod register_action;
mod styles;
mod test;

#[cfg(any(feature = "inspector", debug_assertions))]
mod derive_inspector_reflection;

use proc_macro::TokenStream;
use syn::{DeriveInput, Ident};

/// Derives `gpui::Action` for a type and registers it with the runtime.
///
/// Use it for any type that a keymap or the command palette should be able to
/// name. The type must implement `Clone` and `PartialEq`. Unless the type is a
/// unit struct or `no_json` is set, it must also implement `serde::Deserialize`
/// and `schemars::JsonSchema`, so that keymaps can build it from JSON.
///
/// The doc comment on the type becomes the action's documentation. The
/// `#[action(...)]` attribute sets the name, namespace, aliases and
/// deprecation. See `gpui::Action` for the full attribute reference.
///
/// Use [`register_action!`] if you implement `gpui::Action` by hand.
#[proc_macro_derive(Action, attributes(action))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    derive_action::derive_action(input)
}

/// Registers a type that implements `gpui::Action` by hand.
///
/// Use this when `#[derive(Action)]` doesn't fit, such as when you need a
/// custom `build` or `name`. Pass the type's name, which must be in scope in
/// the module that calls the macro. Call it once per type, in the same crate
/// that defines the type, because the macro adds an inherent `impl` block to
/// the type.
///
/// `register_action!(MyAction);`
#[proc_macro]
pub fn register_action(ident: TokenStream) -> TokenStream {
    register_action::register_action(ident)
}

/// Derives `gpui::IntoElement` for a type that implements `gpui::RenderOnce`.
///
/// The generated `into_element` wraps the value in a `gpui::ViewElement`, which
/// renders it as part of the element tree. This lets you pass the value
/// directly as a child of another element.
#[proc_macro_derive(IntoElement)]
pub fn derive_into_element(input: TokenStream) -> TokenStream {
    derive_into_element::derive_into_element(input)
}

/// Derives `gpui::Styled` for a struct with a `#[style]` field.
///
/// The field must have the type `gpui::StyleRefinement`. The derived `style`
/// method returns that field, so the builder methods such as `flex` and `gap`
/// write to it. Mark at most one field.
#[proc_macro_derive(Styled, attributes(style))]
pub fn derive_styled(input: TokenStream) -> TokenStream {
    derive_element_traits::derive_styled(input)
}

/// Derives `gpui::ParentElement` for a struct with a `#[children]` field.
///
/// The field must be a collection that has an `extend` method that accepts
/// `gpui::AnyElement` values, typically a `Vec<AnyElement>`. Its `child` and
/// `children` methods add to it. Mark at most one field.
#[proc_macro_derive(ParentElement, attributes(children))]
pub fn derive_parent_element(input: TokenStream) -> TokenStream {
    derive_element_traits::derive_parent_element(input)
}

/// Derives `gpui::InteractiveElement` for a struct with an `#[interactivity]`
/// field.
///
/// The field must have the type `gpui::Interactivity`. Mark at most one field.
#[proc_macro_derive(InteractiveElement, attributes(interactivity))]
pub fn derive_interactive_element(input: TokenStream) -> TokenStream {
    derive_element_traits::derive_interactive_element(input)
}

/// Derives `gpui::StatefulInteractiveElement`, which adds methods such as `role`.
///
/// The derive writes an empty impl, so the trait's default methods become available.
/// The type must also implement `gpui::InteractiveElement`, which you can derive with
/// `#[derive(InteractiveElement)]`.
#[proc_macro_derive(StatefulInteractiveElement)]
pub fn derive_stateful_interactive_element(input: TokenStream) -> TokenStream {
    derive_element_traits::derive_stateful_interactive_element(input)
}

/// Derives `gpui::Render` with a `render` method that returns `gpui::Empty`.
///
/// Use it for a view that holds state but draws nothing. Write `impl Render`
/// by hand for any view that draws something.
#[proc_macro_derive(Render)]
#[doc(hidden)]
pub fn derive_render(input: TokenStream) -> TokenStream {
    derive_render::derive_render(input)
}

/// Derives `gpui::AppContext` for a struct that holds a `&mut gpui::App`.
///
/// Mark the field that holds the `&mut App` with `#[app]`. The derived methods
/// forward to that field, so the struct can be used wherever an `AppContext`
/// is accepted. The attribute is required.
///
/// Without the attribute the derive fails to compile:
///
/// ```compile_fail
/// # #[macro_use] extern crate gpui_macros;
/// # #[macro_use] extern crate gpui;
/// #[derive(AppContext)]
/// struct MyContext<'a> {
///     app: &'a mut gpui::App
/// }
/// ```
#[proc_macro_derive(AppContext, attributes(app))]
pub fn derive_app_context(input: TokenStream) -> TokenStream {
    derive_app_context::derive_app_context(input)
}

/// Derives `gpui::VisualContext` for a struct that holds a `&mut gpui::App`
/// and a `&mut gpui::Window`.
///
/// Mark the `&mut App` field with `#[app]` and the `&mut Window` field with
/// `#[window]`. Both attributes are required. The derived methods forward to
/// those fields.
///
/// Failing to add either attribute causes a compile error:
///
/// ```compile_fail
/// # #[macro_use] extern crate gpui_macros;
/// # #[macro_use] extern crate gpui;
/// #[derive(VisualContext)]
/// struct MyContext<'a, 'b> {
///     #[app]
///     app: &'a mut gpui::App,
///     window: &'b mut gpui::Window
/// }
/// ```
///
/// ```compile_fail
/// # #[macro_use] extern crate gpui_macros;
/// # #[macro_use] extern crate gpui;
/// #[derive(VisualContext)]
/// struct MyContext<'a, 'b> {
///     app: &'a mut gpui::App,
///     #[window]
///     window: &'b mut gpui::Window
/// }
/// ```
#[proc_macro_derive(VisualContext, attributes(window, app))]
pub fn derive_visual_context(input: TokenStream) -> TokenStream {
    derive_visual_context::derive_visual_context(input)
}

/// Generates the style helper methods on `Styled`. Used inside `gpui`.
#[proc_macro]
#[doc(hidden)]
pub fn style_helpers(input: TokenStream) -> TokenStream {
    styles::style_helpers(input)
}

/// Generates the style transition builder and its application code. Used inside `gpui`.
#[proc_macro]
#[doc(hidden)]
pub fn style_transitions(input: TokenStream) -> TokenStream {
    styles::style_transitions(input)
}

/// Generates the `visible` and `invisible` methods for `Styled`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn visibility_style_methods(input: TokenStream) -> TokenStream {
    styles::visibility_style_methods(input)
}

/// Generates the margin methods for `Styled`, such as `m_4` and `mx_auto`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn margin_style_methods(input: TokenStream) -> TokenStream {
    styles::margin_style_methods(input)
}

/// Generates the padding methods for `Styled`, such as `p_4` and `px_2`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn padding_style_methods(input: TokenStream) -> TokenStream {
    styles::padding_style_methods(input)
}

/// Generates the position methods for `Styled`, such as `relative` and `absolute`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn position_style_methods(input: TokenStream) -> TokenStream {
    styles::position_style_methods(input)
}

/// Generates the overflow methods for `Styled`, such as `overflow_hidden`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn overflow_style_methods(input: TokenStream) -> TokenStream {
    styles::overflow_style_methods(input)
}

/// Generates the cursor methods for `Styled`, such as `cursor_pointer`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn cursor_style_methods(input: TokenStream) -> TokenStream {
    styles::cursor_style_methods(input)
}

/// Generates the border methods for `Styled`, such as `border_1` and `border_color`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn border_style_methods(input: TokenStream) -> TokenStream {
    styles::border_style_methods(input)
}

/// Generates the box-shadow and corner methods for `Styled`.
///
/// Call it in the body of the `Styled` trait. The optional
/// `{ visibility: pub }` argument sets the visibility of the generated methods.
#[proc_macro]
pub fn box_shadow_style_methods(input: TokenStream) -> TokenStream {
    styles::box_shadow_style_methods(input)
}

/// Marks a function as a GPUI test. The generated function is a `#[test]`, so
/// it runs under `cargo test` or `cargo nextest`.
///
/// The function can be synchronous or `async`. Its parameters decide what it
/// receives:
///
/// - `&mut TestAppContext` (or `&TestAppContext`): a fresh test context. Use one
///   per app you need. Any number is allowed.
/// - `&mut App`: a synchronous test only. The test gets the app directly.
/// - `BackgroundExecutor`: an `async` test only. An executor for the test's
///   scheduler.
/// - `StdRng`: a random number generator seeded with the current run's seed.
///   Use it to vary inputs. Each run sees the same values for the same seed, so
///   changing the seed explores other interleavings.
///
/// Any other parameter type is a compile error.
///
/// An `async` test takes a context, for example
/// `async fn test_counter(cx: &mut TestAppContext)`, and runs inside the test
/// scheduler.
///
/// Your crate needs `rand` as a dev-dependency, because the generated code
/// calls `rand::SeedableRng` to seed a `StdRng` parameter.
///
/// # Arguments
///
/// - `#[gpui::test]` with no arguments runs once, with seed `0` or the value of
///   the `SEED` environment variable.
/// - `#[gpui::test(seed = 10)]` runs once with seed `10`.
/// - `#[gpui::test(seeds(10, 20, 30))]` runs once for each of seeds `10`, `20`
///   and `30`.
/// - `#[gpui::test(iterations = 5)]` runs with seeds `0` to `4`.
/// - `#[gpui::test(retries = 3)]` retries a failing seed up to three more times,
///   so a seed runs at most four times. The retries are per seed.
/// - `#[gpui::test(on_failure = "crate::test::report_failure")]` calls the named
///   function with no arguments once a seed has failed on its final attempt,
///   before the test panics. Use it to write out more details about the failure.
///   The path must be a `fn()`.
///
/// `iterations` combines with `seed` and `seeds`. The iteration seeds run first,
/// then the explicit seeds. For example, `#[gpui::test(iterations = 5, seed = 10)]`
/// runs seeds `0, 1, 2, 3, 4, 10`.
///
/// When a run has more than one seed, the macro prints each seed to stderr. It
/// prints the failing seed too, along with how to rerun it.
///
/// # Environment Variables
///
/// - `SEED`: runs only that seed (or `SEED` through `SEED + iterations - 1` when
///   `iterations` is set). It overrides every explicit seed.
/// - `ITERATIONS`: overrides the value of the `iterations` argument.
#[proc_macro_attribute]
pub fn test(args: TokenStream, function: TokenStream) -> TokenStream {
    test::test(args, function)
}

/// Turns a function into a Criterion benchmark that runs with GPUI support.
///
/// The function takes a `&mut gpui::BenchAppContext`, and it is not `async`.
/// Put the work to measure in the body, and the macro measures it with
/// Criterion. Benchmarks with an `inputs` argument take the input as their
/// first parameter, before the context: `fn bench(input: T, cx: &mut BenchAppContext)`.
///
/// The macro generates a function that takes `&mut criterion::Criterion`, so
/// register it with `criterion_group!` and `criterion_main!` in the benchmark
/// target. Each benchmark prints a GPUI report when it finishes.
///
/// # Arguments
///
/// - `fps = N` sets the frame rate that the report budgets against. `N` must be
///   greater than zero. The default comes from `gpui::BenchReport`.
/// - `inputs = EXPR` runs the benchmark once for each item of `EXPR`, using
///   Criterion's `bench_with_input`.
/// - `input_name = "..."` sets the name of each input in the report. Requires
///   `inputs`. The default is the function name.
/// - `group = "..."` sets the name of the benchmark group. Requires `inputs`.
///   The default is the function name.
/// - `sample_size = N` sets the number of Criterion samples for the group.
///   Requires `inputs`, and `N` must be greater than zero.
///
/// # Dependencies
///
/// The benchmark crate needs `criterion` and `gpui_platform` as dev-dependencies,
/// because the generated code calls both. Enable the `bench-support` feature on
/// `gpui`, which provides `BenchAppContext`.
#[proc_macro_attribute]
pub fn bench(args: TokenStream, function: TokenStream) -> TokenStream {
    bench::bench(args, function)
}

/// A variant of `#[gpui::test]` that runs a test with many generated inputs.
///
/// A property test checks a claim for many generated inputs. For example, a
/// function with `x: i32` and `y: i32` runs for generated pairs of integers;
/// the test body should hold for every pair.
///
/// Standard GPUI randomized tests provide you with an instance of `StdRng` to
/// generate random data in a controlled manner. Property-based tests have some
/// advantages, however:
/// - Shrinking reduces a failing input to a smaller case that still fails.
/// - The harness generates values, so the test body contains only the property.
/// - Failing cases are saved to a file and replayed before new cases.
///
/// Property tests work best when all inputs can be generated up-front and kept
/// in a simple data structure. Sometimes, this isn't possible - for example, if
/// a test needs to make a random decision based on the current state of some
/// structure. In this case, a standard GPUI randomized test may be more
/// suitable.
///
/// ## Customizing random values
///
/// This macro is based on the [`#[proptest::property_test]`] macro, but handles
/// some of the same GPUI-specific arguments as `#[gpui::test]`. Specifically,
/// `&{mut,} TestAppContext` and `BackgroundExecutor` work as normal. `StdRng`
/// arguments are **explicitly forbidden**, since they break shrinking, and are
/// a common footgun.
///
/// All other arguments are forwarded to the underlying proptest macro.
///
/// The code that proptest's macro generates names the `proptest` crate directly, so a crate that
/// uses this macro needs `proptest`, with its `attr-macro` feature, in its dev-dependencies.
///
/// The following terms come from proptest's
/// [`#[proptest::property_test]`] documentation.
///
/// Random values of type `T` are generated by a `Strategy<Value = T>` object.
/// Some types have a canonical `Strategy` - these types also implement
/// `Arbitrary`. Parameters to a `#[gpui::property_test]`, by default, use a
/// type's `Arbitrary` implementation. If you'd like to provide a custom
/// strategy, put `#[strategy = ...]` on the argument. For example,
/// `#[strategy = 1..10] x: i32` draws `x` from the range `1..10`.
///
/// For more information on writing custom `Strategy` and `Arbitrary`
/// implementations, see [the proptest book][book], and the [`Strategy`] trait.
///
/// ## Scheduler
///
/// Each case receives a scheduler seed, which the macro supplies as a parameter
/// that doesn't shrink, because all seeds are equally simple. Without `SEED`,
/// the case seeds come from a proptest generator fixed at `0`, so runs are
/// reproducible. If `SEED` is set, every case uses that value as its scheduler
/// seed, and the proptest generator is seeded with it too. To control other
/// values, use custom `Strategy`s.
///
/// [`#[proptest::property_test]`]: https://docs.rs/proptest/latest/proptest/attr.property_test.html
/// [book]: https://proptest-rs.github.io/proptest/intro.html
/// [`Strategy`]: https://docs.rs/proptest/latest/proptest/strategy/trait.Strategy.html
#[proc_macro_attribute]
pub fn property_test(args: TokenStream, function: TokenStream) -> TokenStream {
    property_test::test(args.into(), function.into()).into()
}

/// When added to a trait, `#[derive_inspector_reflection]` generates a module which provides
/// enumeration and lookup by name of all methods that have the shape `fn method(self) -> Self`.
/// This is used by the inspector so that it can use the builder methods in `Styled` and
/// `StyledExt`.
///
/// The generated module is named `<snake_case_trait_name>_reflection`. It has two
/// generic functions, `methods::<T>()`, which returns every reflected method
/// as a `Vec<gpui::inspector_reflection::FunctionReflection<T>>`, and
/// `find_method::<T>(name)`, which looks one up by name and returns an
/// `Option` of the same type. `T` must implement the trait and be `'static`.
///
/// The `invoke` method on `FunctionReflection` runs the method. `FunctionReflection`
/// also provides the method's documentation.
///
/// This attribute is only compiled with the `inspector` feature or in debug builds.
#[cfg(any(feature = "inspector", debug_assertions))]
#[proc_macro_attribute]
pub fn derive_inspector_reflection(_args: TokenStream, input: TokenStream) -> TokenStream {
    derive_inspector_reflection::derive_inspector_reflection(_args, input)
}

pub(crate) fn get_simple_attribute_field(ast: &DeriveInput, name: &'static str) -> Option<Ident> {
    match &ast.data {
        syn::Data::Struct(data_struct) => data_struct
            .fields
            .iter()
            .find(|field| field.attrs.iter().any(|attr| attr.path().is_ident(name)))
            .map(|field| field.ident.clone().unwrap()),
        syn::Data::Enum(_) => None,
        syn::Data::Union(_) => None,
    }
}
