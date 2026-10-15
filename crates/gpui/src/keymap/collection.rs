use crate::{Action, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, SharedString};
use std::{any::TypeId, collections::HashMap};

/// A set of actions and the keystrokes that trigger each one, to be bound together in one key
/// context.
///
/// Use it when one action has several keystrokes, or when a group of bindings shares a context.
/// Build it with [`with`](Self::with), then turn it into [`KeyBinding`]s with
/// [`as_keybindings`](Self::as_keybindings) and pass them to
/// [`App::bind_keys`](crate::App::bind_keys).
///
/// # Examples
///
/// ```standalone_crate
/// use gpui::{ActionBindingCollection, actions};
///
/// actions!(collection_docs, [Submit]);
///
/// let bindings = ActionBindingCollection::default()
///     .with::<Submit>("enter")
///     .with::<Submit>("ctrl-enter");
///
/// // One binding per keystroke, all in the `Form` context.
/// let bindings: Vec<_> = bindings.as_keybindings(Some("Form")).collect();
/// assert_eq!(bindings.len(), 2);
/// ```
#[derive(Default)]
pub struct ActionBindingCollection {
    entries: HashMap<TypeId, ActionBindingEntry>,
}

#[derive(Debug)]
struct ActionBindingEntry {
    // The action used to build Keybinding
    action: Box<dyn Action>,
    // The keystrokes that trigger the action
    // NOTE: could limit allocations by utilizing smallvec
    keystrokes: Vec<SharedString>,
}

impl ActionBindingCollection {
    /// Adds `keystrokes` as a trigger for the action type `A`, and returns the collection.
    ///
    /// The action is built with `A::default()` once per type, so `A` must be a type you can
    /// construct with `Default`, such as an action declared with [`actions!`](crate::actions).
    /// Keystrokes aren't parsed here. They are parsed by [`as_keybindings`](Self::as_keybindings),
    /// which panics on an invalid one.
    pub fn with<A: Action + Default>(mut self, keystrokes: impl Into<SharedString>) -> Self {
        let action_id = TypeId::of::<A>();
        let entry = self.entries.entry(action_id);
        let entry = entry.or_insert_with(|| ActionBindingEntry {
            action: Box::new(A::default()),
            keystrokes: Vec::with_capacity(5),
        });
        entry.keystrokes.push(keystrokes.into());
        self
    }

    /// Returns one [`KeyBinding`] for each action and keystroke in the collection, all in `context`.
    ///
    /// Pass `None` for `context` to make the bindings apply everywhere. Elements that declare a
    /// matching [`key_context`](crate::InteractiveElement::key_context) receive the actions.
    /// The order of the bindings is unspecified, because the collection is a hash map keyed by
    /// action type. Bindings for different actions on the same keystrokes therefore have no
    /// defined precedence.
    ///
    /// # Panics
    ///
    /// Panics if a keystroke fails to parse, or if `context` is not a valid key context
    /// predicate.
    pub fn as_keybindings(&self, context: Option<&str>) -> impl Iterator<Item = KeyBinding> {
        let context_predicate =
            context.map(|context| KeyBindingContextPredicate::parse(context).unwrap().into());
        let iter = self.entries.iter();
        let iter = iter.map(move |(_, entry)| {
            let action = &entry.action;
            let context_predicate = context_predicate.clone();
            entry.keystrokes.iter().map(move |keystrokes| {
                let keybinding = KeyBinding::load(
                    keystrokes.as_str(),
                    action.boxed_clone(),
                    context_predicate.clone(),
                    false,
                    None,
                    &DummyKeyboardMapper,
                );
                keybinding.expect("failed to load keybinding")
            })
        });
        iter.flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestAppContext;

    actions!([TestAction]);

    #[gpui::test]
    fn test_default_empty(_cx: &mut TestAppContext) {
        assert!(ActionBindingCollection::default().entries.is_empty());
    }

    #[gpui::test]
    fn test_single_binding(_cx: &mut TestAppContext) {
        let collection = ActionBindingCollection::default();
        let collection = collection.with::<TestAction>("enter");

        let entry = collection.entries.get(&TypeId::of::<TestAction>());
        let entry = entry.map(|entry| &entry.keystrokes);
        assert_eq!(entry, Some(&vec![SharedString::new_static("enter")]));

        let out_bindings = collection.as_keybindings(None).collect::<Vec<_>>();
        assert_eq!(out_bindings.len(), 1);
    }

    #[gpui::test]
    fn test_single_action_many_keys(_cx: &mut TestAppContext) {
        let collection = ActionBindingCollection::default();
        let collection = collection.with::<TestAction>("enter");
        let collection = collection.with::<TestAction>("escape");

        let entry = collection.entries.get(&TypeId::of::<TestAction>());
        let entry = entry.map(|entry| &entry.keystrokes);
        assert_eq!(
            entry,
            Some(&vec![
                SharedString::new_static("enter"),
                SharedString::new_static("escape")
            ])
        );

        let out_bindings = collection.as_keybindings(None).collect::<Vec<_>>();
        assert_eq!(out_bindings.len(), 2);
    }

    #[gpui::test]
    fn test_single_action_many_keys_context(_cx: &mut TestAppContext) {
        let collection = ActionBindingCollection::default();
        let collection = collection.with::<TestAction>("enter");
        let collection = collection.with::<TestAction>("escape");
        let out_bindings = collection
            .as_keybindings(Some("test_content"))
            .collect::<Vec<_>>();
        assert_eq!(out_bindings.len(), 2);
    }
}
