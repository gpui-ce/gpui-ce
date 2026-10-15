use std::rc::Rc;

use crate::{
    Action, AsKeystroke, DummyKeyboardMapper, InvalidKeystrokeError, KeyBindingContextPredicate,
    KeybindingKeystroke, Keystroke, PlatformKeyboardMapper, SharedString,
};
use smallvec::SmallVec;

/// A keybinding: a keystroke sequence, the action it runs, and the key context it applies in.
///
/// You usually create one with [`KeyBinding::new`] and register it with
/// [`App::bind_keys`](crate::App::bind_keys). Its accessors let you inspect a binding, for
/// example to show its shortcut in a menu:
///
/// ```standalone_crate
/// use gpui::{Action, KeyBinding, actions};
///
/// actions!(binding_docs, [Save]);
///
/// let binding = KeyBinding::new("cmd-k cmd-s", Save, Some("Editor"));
/// assert_eq!(binding.keystrokes().len(), 2);
/// assert_eq!(binding.action().name(), "binding_docs::Save");
/// assert!(binding.predicate().is_some());
/// ```
///
/// See [keyboard input](crate::_documentation::concepts::input_dispatch) for how bindings are
/// matched against the focus path.
pub struct KeyBinding {
    pub(crate) action: Box<dyn Action>,
    pub(crate) keystrokes: SmallVec<[KeybindingKeystroke; 2]>,
    pub(crate) context_predicate: Option<Rc<KeyBindingContextPredicate>>,
    pub(crate) meta: Option<KeyBindingMetaIndex>,
    /// The json input string used when building the keybinding, if any
    pub(crate) action_input: Option<SharedString>,
}

impl Clone for KeyBinding {
    fn clone(&self) -> Self {
        KeyBinding {
            action: self.action.boxed_clone(),
            keystrokes: self.keystrokes.clone(),
            context_predicate: self.context_predicate.clone(),
            meta: self.meta,
            action_input: self.action_input.clone(),
        }
    }
}

impl KeyBinding {
    /// Creates a binding that runs `action` when `keystrokes` is typed in `context`.
    ///
    /// `keystrokes` is a whitespace-separated sequence of
    /// [keystrokes](Keystroke::parse), such as `"cmd-s"` or `"ctrl-k ctrl-s"`.
    /// `context` is a [key context predicate](KeyBindingContextPredicate::parse),
    /// such as `"Editor"` or `"Editor && mode == insert"`. Pass `None` to make the binding apply
    /// everywhere, which counts as the deepest context for precedence.
    ///
    /// Bindings made here use the keys as written. They never go through the platform's keyboard
    /// mapping, so use this for bindings in code.
    ///
    /// # Panics
    ///
    /// Panics if any keystroke or the context fails to parse. Use [`KeyBinding::load`] to get an
    /// error instead.
    ///
    /// # Examples
    ///
    /// ```standalone_crate
    /// use gpui::{KeyBinding, actions};
    ///
    /// actions!(binding_new_docs, [Save]);
    ///
    /// let binding = KeyBinding::new("secondary-s", Save, Some("Editor"));
    /// assert_eq!(binding.keystrokes().len(), 1);
    /// ```
    pub fn new<A: Action>(keystrokes: &str, action: A, context: Option<&str>) -> Self {
        let context_predicate =
            context.map(|context| KeyBindingContextPredicate::parse(context).unwrap().into());
        Self::load(
            keystrokes,
            Box::new(action),
            context_predicate,
            false,
            None,
            &DummyKeyboardMapper,
        )
        .unwrap()
    }

    /// Creates a binding from the parts that a keymap file provides.
    ///
    /// `keystrokes` is split on spaces and each part is parsed with [`Keystroke::parse`]. Each
    /// keystroke is then passed to `keyboard_mapper`, and `use_key_equivalents` asks it for the
    /// layout-dependent mapping. On macOS, that replaces a key with the character the current
    /// layout types for it. For bindings from a keymap file, `action_input` is the JSON the action
    /// was built from. Pass `None` when there is none.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidKeystrokeError`] if any keystroke fails to parse. The context is already
    /// parsed, so this function doesn't check it.
    pub fn load(
        keystrokes: &str,
        action: Box<dyn Action>,
        context_predicate: Option<Rc<KeyBindingContextPredicate>>,
        use_key_equivalents: bool,
        action_input: Option<SharedString>,
        keyboard_mapper: &dyn PlatformKeyboardMapper,
    ) -> std::result::Result<Self, InvalidKeystrokeError> {
        let keystrokes: SmallVec<[KeybindingKeystroke; 2]> = keystrokes
            .split_whitespace()
            .map(|source| {
                let keystroke = Keystroke::parse(source)?;
                Ok(KeybindingKeystroke::new_with_mapper(
                    keystroke,
                    use_key_equivalents,
                    keyboard_mapper,
                ))
            })
            .collect::<std::result::Result<_, _>>()?;

        Ok(Self {
            keystrokes,
            action,
            context_predicate,
            meta: None,
            action_input,
        })
    }

    /// Sets the source metadata for this binding and returns it.
    ///
    /// See [`KeyBindingMetaIndex`] for how the keymap uses this. A binding without metadata
    /// counts as the strongest source.
    pub fn with_meta(mut self, meta: KeyBindingMetaIndex) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Sets the source metadata for this binding. See [`KeyBinding::with_meta`].
    pub fn set_meta(&mut self, meta: KeyBindingMetaIndex) {
        self.meta = Some(meta);
    }

    /// Returns how the typed `keystrokes` relate to this binding's keystroke sequence.
    ///
    /// Returns `None` if they don't match. Returns `Some(true)` if they match the start of the
    /// sequence and more keystrokes are needed, and `Some(false)` if they match all of it. The
    /// keymap uses this to decide whether to wait for the next key.
    ///
    /// # Examples
    ///
    /// ```standalone_crate
    /// use gpui::{Keystroke, KeyBinding, actions};
    ///
    /// actions!(match_docs, [Save]);
    ///
    /// let binding = KeyBinding::new("ctrl-k ctrl-s", Save, None);
    /// let ctrl_k = [Keystroke::parse("ctrl-k").unwrap()];
    /// let ctrl_k_ctrl_s = [
    ///     Keystroke::parse("ctrl-k").unwrap(),
    ///     Keystroke::parse("ctrl-s").unwrap(),
    /// ];
    ///
    /// assert_eq!(binding.match_keystrokes(&ctrl_k), Some(true));
    /// assert_eq!(binding.match_keystrokes(&ctrl_k_ctrl_s), Some(false));
    /// assert_eq!(binding.match_keystrokes(&[Keystroke::parse("ctrl-x").unwrap()]), None);
    /// ```
    pub fn match_keystrokes(&self, typed: &[impl AsKeystroke]) -> Option<bool> {
        if self.keystrokes.len() < typed.len() {
            return None;
        }

        for (target, typed) in self.keystrokes.iter().zip(typed.iter()) {
            if !typed.as_keystroke().should_match(target) {
                return None;
            }
        }

        Some(self.keystrokes.len() > typed.len())
    }

    /// Returns the keystrokes that trigger this binding, in order. A binding has one keystroke
    /// for a single key, and more for a sequence such as `ctrl-k ctrl-s`.
    pub fn keystrokes(&self) -> &[KeybindingKeystroke] {
        self.keystrokes.as_slice()
    }

    /// Returns the action this binding runs.
    pub fn action(&self) -> &dyn Action {
        self.action.as_ref()
    }

    /// Returns the key context predicate this binding applies in, or `None` if it applies in
    /// every context.
    pub fn predicate(&self) -> Option<Rc<KeyBindingContextPredicate>> {
        self.context_predicate.as_ref().map(|rc| rc.clone())
    }

    /// Returns the source metadata set with [`KeyBinding::with_meta`], if any.
    pub fn meta(&self) -> Option<KeyBindingMetaIndex> {
        self.meta
    }

    /// Returns the JSON input the action was built from, if the binding came from a keymap file.
    pub fn action_input(&self) -> Option<SharedString> {
        self.action_input.clone()
    }
}

impl std::fmt::Debug for KeyBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyBinding")
            .field("keystrokes", &self.keystrokes)
            .field("context_predicate", &self.context_predicate)
            .field("action", &self.action.name())
            .finish()
    }
}

/// An index into your own store of metadata about where a binding came from, such as the keymap
/// file or the built-in defaults. Lower indexes are stronger sources.
///
/// The keymap uses the index when a [`NoAction`](crate::NoAction) disables a lower-precedence
/// binding for the same keystrokes. The `NoAction` disables bindings from its own source or a
/// weaker one (a higher index), but not bindings from a stronger source. A binding without an
/// index counts as index `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyBindingMetaIndex(pub u32);
