use crate::{Action, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, SharedString};
use std::{any::TypeId, collections::HashMap};

/// A generic collection of `Action` -> keystrokes which can be bound under a shared key-context.
/// Use `as_keybindings` to create an iterator of `KeyBinding` to provide to `App::bind_keys`.
///
/// ```rust
/// # use gpui::{ActionBindingCollection, Action};
/// gpui::actions!([MyAction]);
/// # #[gpui::test]
/// # fn test(app: &mut gpui::TestAppContext) {
/// app.bind_keys(ActionBindingCollection::default().with::<MyAction>("enter").as_keybindings(None));
/// # }
/// ```
#[derive(Default)]
pub struct ActionBindingCollection {
    entries: HashMap<TypeId, ActionBindingEntry>,
}

#[derive(Debug)]
struct ActionBindingEntry {
    // The action used to build Keybinding
    action: Box<dyn Action>,
    // Triggering keystrokes. TODO: Use SmallVec to reduce allocations.
    keystrokes: Vec<SharedString>,
}

impl ActionBindingCollection {
    /// Adds a keystroke to an action in the collection.
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

    /// Creates an iterator of `KeyBinding` which represents all action+keystrokes in this collection bound to the same key-context.
    /// This sequence can be provided to `App::bind_keys`, and any elements which use the same `InteractiveElement::key_context` will receive the bound actions.
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

    actions!([TestAction]);

    #[test]
    fn test_action_bindings() {
        let mut collection = ActionBindingCollection::default();
        assert!(collection.entries.is_empty());

        for (key, expected) in [
            ("enter", &["enter"][..]),
            ("escape", &["enter", "escape"][..]),
        ] {
            collection = collection.with::<TestAction>(key);

            let entry = collection.entries.get(&TypeId::of::<TestAction>()).unwrap();
            assert_eq!(entry.keystrokes, expected);

            let bindings = collection.as_keybindings(None).collect::<Vec<_>>();
            assert_eq!(bindings.len(), expected.len());
        }

        let bindings = collection
            .as_keybindings(Some("test_content"))
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), 2);
    }
}
