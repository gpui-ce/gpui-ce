use anyhow::{Context as _, Result};
use collections::{HashMap, TypeIdHashMap};
pub use gpui_macros::Action;
pub use no_action::{NoAction, Unbind, is_no_action, is_unbind};
use serde_json::json;
use std::{
    any::{Any, TypeId},
    fmt::Display,
};

/// Declares unit structs that are actions, and registers them with the app.
///
/// Each name becomes a struct that implements [`Action`]. Use this for actions that take no data.
/// For actions with fields, use `#[derive(Action)]` instead.
///
/// The structs derive `Clone`, `PartialEq`, `Default` and `Debug`. Attributes written before a
/// name, such as a doc comment, are passed on to the struct. A doc comment becomes the action's
/// [`documentation`](Action::documentation).
///
/// With a namespace, the action's name is `namespace::Name`; without one, it is `Name`.
/// Two actions with the same name make app creation panic.
///
/// # Examples
///
/// ```standalone_crate
/// use gpui::{Action, actions};
///
/// actions!(editor_macro_docs, [MoveUp, MoveDown, MoveLeft, MoveRight, Newline]);
///
/// assert_eq!(MoveUp.name(), "editor_macro_docs::MoveUp");
/// ```
///
#[macro_export]
macro_rules! actions {
    ($namespace:path, [ $( $(#[$attr:meta])* $name:ident),* $(,)? ]) => {
        $(
            #[derive(::std::clone::Clone, ::std::cmp::PartialEq, ::std::default::Default, ::std::fmt::Debug, gpui::Action)]
            #[action(namespace = $namespace)]
            $(#[$attr])*
            pub struct $name;
        )*
    };
    ([ $( $(#[$attr:meta])* $name:ident),* $(,)? ]) => {
        $(
            #[derive(::std::clone::Clone, ::std::cmp::PartialEq, ::std::default::Default, ::std::fmt::Debug, gpui::Action)]
            $(#[$attr])*
            pub struct $name;
        )*
    };
}

/// An action is a named command that the app can run, such as saving or moving the cursor up.
///
/// Declaring an action separates what the app can do from the keys that trigger it. A
/// [`KeyBinding`](crate::KeyBinding) maps keystrokes to an action, a menu or button can run the
/// same action, and [`InteractiveElement::on_action`](crate::InteractiveElement::on_action)
/// handlers receive it. See [keyboard input](crate::_documentation::concepts::input_dispatch) for
/// the dispatch order.
///
/// Declare unit actions with [`actions!`](crate::actions), and actions with data with
/// `#[derive(Action)]`. The derive macro implements every method of this trait and registers the
/// action by name, so you rarely write an impl yourself.
///
/// # Examples
///
/// This action has a field, so the keymap entry carries a value. The handler runs when `ctrl-r`
/// is typed while the `Counter` element has focus:
///
/// ```standalone_crate
/// use gpui::{Action, Context, FocusHandle, KeyBinding, TestApp, Window, div, prelude::*};
///
/// #[derive(Clone, PartialEq, serde::Deserialize, schemars::JsonSchema, Action)]
/// #[action(namespace = action_trait_docs)]
/// pub struct Repeat {
///     pub times: usize,
/// }
///
/// struct Counter {
///     focus_handle: FocusHandle,
///     total: usize,
/// }
///
/// impl Render for Counter {
///     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
///         div()
///             .key_context("Counter")
///             .track_focus(&self.focus_handle)
///             .on_action(cx.listener(|counter, action: &Repeat, _window, _cx| {
///                 counter.total += action.times;
///             }))
///     }
/// }
///
/// let mut app = TestApp::new();
/// app.update(|cx| cx.bind_keys([KeyBinding::new("ctrl-r", Repeat { times: 2 }, Some("Counter"))]));
///
/// let mut window = app.open_window(|window, cx| {
///     let focus_handle = cx.focus_handle();
///     window.focus(&focus_handle, cx);
///     Counter { focus_handle, total: 0 }
/// });
///
/// window.simulate_keystrokes("ctrl-r");
/// assert_eq!(window.read(|counter, _| counter.total), 2);
/// ```
///
/// # Derive attributes
///
/// `#[derive(Action)]` requires `Clone` and `PartialEq`. A struct with fields also requires
/// `serde::Deserialize` and `schemars::JsonSchema`, which build the action from JSON and describe
/// its shape. A unit struct, or a type with `no_json`, needs neither.
///
/// Options go in `#[action(...)]`, separated by commas:
///
/// - `namespace = ident` prefixes the name, so `namespace = editor` on `Save` gives
///   `editor::Save`. The value is an identifier, not a string.
/// - `name = "Text"` sets the name without a namespace. The name must not contain `::`.
/// - `no_json` makes [`Action::build`] always return an error and makes
///   [`Action::action_json_schema`] return `None`. The type then doesn't need `Deserialize` or
///   `JsonSchema`.
/// - `no_register` skips registration. The action can't be invoked by name or built from JSON.
/// - `deprecated_aliases = ["editor::OldName"]` keeps old names working. Each old name must not be
///   the name of a registered action.
/// - `deprecated = "Message"` attaches a deprecation message, which [`Action::deprecation_message`]
///   returns.
///
/// # Manual implementation
///
/// To control every method yourself, implement the trait and call
/// [`register_action!`](crate::register_action!). It registers the action without the rest of the
/// derive.
///
/// ```standalone_crate
/// use gpui::{SharedString, register_action};
/// #[derive(Clone, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
/// pub struct Paste {
///     pub content: SharedString,
/// }
///
/// impl gpui::Action for Paste {
///     # fn boxed_clone(&self) -> Box<dyn gpui::Action> { unimplemented!()}
///     # fn partial_eq(&self, other: &dyn gpui::Action) -> bool { unimplemented!() }
///     # fn name(&self) -> &'static str { "Paste" }
///     # fn name_for_type() -> &'static str { "Paste" }
///     # fn build(value: serde_json::Value) -> anyhow::Result<Box<dyn gpui::Action>> {
///     #     unimplemented!()
///     # }
/// }
///
/// register_action!(Paste);
/// ```
pub trait Action: Any + Send {
    /// Returns a boxed copy of this action.
    ///
    /// Keymaps clone their bindings with this, so each binding owns its own action value.
    fn boxed_clone(&self) -> Box<dyn Action>;

    /// Returns whether `action` is the same action type as `self` and has equal fields.
    ///
    /// Returns `false` for an action of a different type.
    fn partial_eq(&self, action: &dyn Action) -> bool;

    /// Returns the action's full name, such as `editor::MoveUp`.
    ///
    /// Keymap files refer to actions by this name, and the name is what [`App::build_action`](crate::App::build_action)
    /// accepts.
    fn name(&self) -> &'static str;

    /// Returns the same name as [`name`](Action::name), for the type rather than a value.
    fn name_for_type() -> &'static str
    where
        Self: Sized;

    /// Builds this action from JSON, as a keymap entry supplies it.
    ///
    /// `value` is `{}` for an action that takes no input. The derive macro reads `value` with
    /// `serde`, so the JSON must match the type's fields. Returns an error if it doesn't match, and
    /// always returns an error for an action declared with `no_json`. To build an action by name,
    /// use [`App::build_action`](crate::App::build_action).
    fn build(value: serde_json::Value) -> Result<Box<dyn Action>>
    where
        Self: Sized;

    /// Returns the JSON schema of the input this action accepts, or `None` if it takes no input.
    ///
    /// The derive macro fills this in from `schemars::JsonSchema`. Schema generation for keymap
    /// files uses it.
    fn action_json_schema(_: &mut schemars::SchemaGenerator) -> Option<schemars::Schema>
    where
        Self: Sized,
    {
        None
    }

    /// Returns the old names that still invoke this action. They are set with the
    /// `deprecated_aliases` option of `#[action(...)]`.
    fn deprecated_aliases() -> &'static [&'static str]
    where
        Self: Sized,
    {
        &[]
    }

    /// Returns the deprecation message, or `None` if the action isn't deprecated. It is set with
    /// the `deprecated` option of `#[action(...)]`.
    fn deprecation_message() -> Option<&'static str>
    where
        Self: Sized,
    {
        None
    }

    /// Returns the action's documentation, or `None` if it has none.
    ///
    /// The derive macro and [`actions!`](crate::actions) set this from the doc comments on the
    /// type.
    fn documentation() -> Option<&'static str>
    where
        Self: Sized,
    {
        None
    }
}

impl std::fmt::Debug for dyn Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("dyn Action")
            .field("name", &self.name())
            .finish()
    }
}

impl dyn Action {
    /// Returns this action as `&dyn Any`, so that you can downcast it to its concrete type with
    /// `downcast_ref`.
    pub fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

/// Error returned by [`App::build_action`](crate::App::build_action) when an action can't be
/// built by name.
///
/// It is a plain error type rather than `anyhow::Error`.
#[derive(Debug)]
pub enum ActionBuildError {
    /// No action with this name is registered.
    NotFound {
        /// The name that was not found.
        name: String,
    },
    /// An action with this name exists, but building it failed. This is usually a JSON
    /// deserialization error, such as a missing field.
    BuildError {
        /// The name of the action that failed to build.
        name: String,
        /// The error from building the action.
        error: anyhow::Error,
    },
}

impl std::error::Error for ActionBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ActionBuildError::NotFound { .. } => None,
            ActionBuildError::BuildError { error, .. } => error.source(),
        }
    }
}

impl Display for ActionBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionBuildError::NotFound { name } => {
                write!(f, "Didn't find an action named \"{name}\"")
            }
            ActionBuildError::BuildError { name, error } => {
                write!(f, "Error while building action \"{name}\": {error}")
            }
        }
    }
}

type ActionBuilder = fn(json: serde_json::Value) -> anyhow::Result<Box<dyn Action>>;

/// Every action registered with the app, indexed by name and by type.
pub(crate) struct ActionRegistry {
    by_name: HashMap<&'static str, ActionData>,
    names_by_type_id: TypeIdHashMap<&'static str>,
    all_names: Vec<&'static str>, // So we can return a static slice.
    deprecated_aliases: HashMap<&'static str, &'static str>, // deprecated name -> preferred name
    deprecation_messages: HashMap<&'static str, &'static str>, // action name -> deprecation message
    documentation: HashMap<&'static str, &'static str>, // action name -> documentation
}

impl Default for ActionRegistry {
    fn default() -> Self {
        let mut this = ActionRegistry {
            by_name: Default::default(),
            names_by_type_id: Default::default(),
            documentation: Default::default(),
            all_names: Default::default(),
            deprecated_aliases: Default::default(),
            deprecation_messages: Default::default(),
        };

        this.load_actions();

        this
    }
}

struct ActionData {
    pub build: ActionBuilder,
    pub json_schema: fn(&mut schemars::SchemaGenerator) -> Option<schemars::Schema>,
}

/// This type must be public so that our macros can build it in other crates.
/// But this is an implementation detail and should not be used directly.
#[doc(hidden)]
pub struct MacroActionBuilder(pub fn() -> MacroActionData);

/// This type must be public so that our macros can build it in other crates.
/// But this is an implementation detail and should not be used directly.
#[doc(hidden)]
pub struct MacroActionData {
    pub name: &'static str,
    pub type_id: TypeId,
    pub build: ActionBuilder,
    pub json_schema: fn(&mut schemars::SchemaGenerator) -> Option<schemars::Schema>,
    pub deprecated_aliases: &'static [&'static str],
    pub deprecation_message: Option<&'static str>,
    pub documentation: Option<&'static str>,
}

inventory::collect!(MacroActionBuilder);

impl ActionRegistry {
    /// Load all registered actions into the registry.
    pub(crate) fn load_actions(&mut self) {
        for builder in inventory::iter::<MacroActionBuilder> {
            let action = builder.0();
            self.insert_action(action);
        }
    }

    fn insert_action(&mut self, action: MacroActionData) {
        let name = action.name;
        if self.by_name.contains_key(name) {
            panic!(
                "Action with name `{name}` already registered \
                (might be registered in `#[action(deprecated_aliases = [...])]`."
            );
        }
        self.by_name.insert(
            name,
            ActionData {
                build: action.build,
                json_schema: action.json_schema,
            },
        );
        for &alias in action.deprecated_aliases {
            if self.by_name.contains_key(alias) {
                panic!(
                    "Action with name `{alias}` already registered. \
                    `{alias}` is specified in `#[action(deprecated_aliases = [...])]` for action `{name}`."
                );
            }
            self.by_name.insert(
                alias,
                ActionData {
                    build: action.build,
                    json_schema: action.json_schema,
                },
            );
            self.deprecated_aliases.insert(alias, name);
            self.all_names.push(alias);
        }
        self.names_by_type_id.insert(action.type_id, name);
        self.all_names.push(name);
        if let Some(deprecation_msg) = action.deprecation_message {
            self.deprecation_messages.insert(name, deprecation_msg);
        }
        if let Some(documentation) = action.documentation {
            self.documentation.insert(name, documentation);
        }
    }

    /// Builds the action registered for `type_id`, with no input.
    pub fn build_action_type(&self, type_id: &TypeId) -> Result<Box<dyn Action>> {
        let name = self
            .names_by_type_id
            .get(type_id)
            .with_context(|| format!("no action type registered for {type_id:?}"))?;

        Ok(self.build_action(name, None)?)
    }

    #[cfg(feature = "profiler")]
    pub(crate) fn try_resolve_action(&self, type_id: &TypeId) -> Option<&'static str> {
        self.names_by_type_id.get(type_id).copied()
    }

    /// Builds the action called `name` from optional JSON `params`, using `{}` when there are none.
    pub fn build_action(
        &self,
        name: &str,
        params: Option<serde_json::Value>,
    ) -> std::result::Result<Box<dyn Action>, ActionBuildError> {
        let build_action = self
            .by_name
            .get(name)
            .ok_or_else(|| ActionBuildError::NotFound {
                name: name.to_owned(),
            })?
            .build;
        (build_action)(params.unwrap_or_else(|| json!({}))).map_err(|e| {
            ActionBuildError::BuildError {
                name: name.to_owned(),
                error: e,
            }
        })
    }

    /// Returns every registered action name, including deprecated aliases.
    pub fn all_action_names(&self) -> &[&'static str] {
        self.all_names.as_slice()
    }

    /// Returns each action name with its JSON schema, in registration order.
    pub fn action_schemas(
        &self,
        generator: &mut schemars::SchemaGenerator,
    ) -> Vec<(&'static str, Option<schemars::Schema>)> {
        // Use the order from all_names so that the resulting schema has sensible order.
        self.all_names
            .iter()
            .map(|name| {
                let action_data = self
                    .by_name
                    .get(name)
                    .expect("All actions in all_names should be registered");
                (*name, (action_data.json_schema)(generator))
            })
            .collect::<Vec<_>>()
    }

    /// Returns the JSON schema of the action called `name`, or `None` if no action has that name.
    pub fn action_schema_by_name(
        &self,
        name: &str,
        generator: &mut schemars::SchemaGenerator,
    ) -> Option<Option<schemars::Schema>> {
        self.by_name
            .get(name)
            .map(|action_data| (action_data.json_schema)(generator))
    }

    /// Maps each deprecated alias to the action name it replaces.
    pub fn deprecated_aliases(&self) -> &HashMap<&'static str, &'static str> {
        &self.deprecated_aliases
    }

    /// Maps each deprecated action name to its deprecation message.
    pub fn deprecation_messages(&self) -> &HashMap<&'static str, &'static str> {
        &self.deprecation_messages
    }

    /// Maps each action name to its documentation.
    pub fn documentation(&self) -> &HashMap<&'static str, &'static str> {
        &self.documentation
    }
}

/// Generate a list of all the registered actions.
/// Useful for transforming the list of available actions into a
/// format suited for static analysis such as in validating keymaps, or
/// generating documentation.
pub fn generate_list_of_all_registered_actions() -> impl Iterator<Item = MacroActionData> {
    inventory::iter::<MacroActionBuilder>
        .into_iter()
        .map(|builder| builder.0())
}

mod no_action {
    use crate as gpui;
    use schemars::JsonSchema;
    use serde::Deserialize;

    actions!(
        zed,
        [
            /// Disables the lower-precedence bindings for the same keystrokes.
            ///
            /// When a `NoAction` binding is the highest-precedence match for some keystrokes, the
            /// bindings that rank below it for those keystrokes are ignored. Bindings from a
            /// stronger source, such as user settings over defaults, still apply. Use it to turn
            /// off a shortcut in one context without touching the binding in other contexts.
            ///
            /// See [`Keymap::bindings_for_input`](crate::Keymap::bindings_for_input) for the rule.
            NoAction
        ]
    );

    /// Removes a lower-precedence binding for the same keystrokes that runs the named action.
    ///
    /// Give the action's full name, such as `"editor::NewLine"`, as the field. When an `Unbind`
    /// binding is the highest-precedence match, a later binding for the same keystrokes that runs
    /// the named action is ignored, whatever context that binding is in. Unlike [`NoAction`], it
    /// leaves other actions on the same keystrokes alone.
    ///
    /// Its input is the target name, such as `"editor::NewLine"`.
    #[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema, gpui::Action)]
    #[action(namespace = zed)]
    pub struct Unbind(pub gpui::SharedString);

    /// Returns whether `action` is a [`NoAction`].
    pub fn is_no_action(action: &dyn gpui::Action) -> bool {
        action.as_any().is::<NoAction>()
    }

    /// Returns whether `action` is an [`Unbind`].
    pub fn is_unbind(action: &dyn gpui::Action) -> bool {
        action.as_any().is::<Unbind>()
    }
}
