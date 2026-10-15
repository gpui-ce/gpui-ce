use collections::HashMap;

use crate::{KeybindingKeystroke, Keystroke};

/// The keyboard layout that is currently active on a platform.
///
/// Each platform implements this to describe its active keyboard layout. The identifier and name
/// are whatever the platform reports.
pub trait PlatformKeyboardLayout {
    /// Returns the layout's identifier. It is unique to the layout, so compare identifiers to
    /// detect a change.
    fn id(&self) -> &str;
    /// Returns the layout's name for display, such as `U.S.`.
    fn name(&self) -> &str;
}

/// Maps keystrokes in keybindings to the keys the platform reports.
///
/// The platform implements this to handle keyboard layouts where a binding's key is typed
/// through another key. See [`KeyBinding::load`](crate::KeyBinding::load).
pub trait PlatformKeyboardMapper {
    /// Maps `keystroke` from a keybinding to the form the platform matches against.
    ///
    /// `use_key_equivalents` asks for the mapping that depends on the current layout. The
    /// implementation decides what that means. The mapping is applied to keybindings loaded with
    /// [`KeyBinding::load`](crate::KeyBinding::load).
    fn map_key_equivalent(
        &self,
        keystroke: Keystroke,
        use_key_equivalents: bool,
    ) -> KeybindingKeystroke;
    /// Returns the key equivalents for the current layout, mapping each character to the one
    /// that the layout types in its place. Returns `None` where the platform has no such table.
    ///
    /// macOS returns one for the layouts it has a table for. Windows returns `None`.
    fn get_key_equivalents(&self) -> Option<&HashMap<char, char>>;
}

/// A [`PlatformKeyboardMapper`] that changes nothing.
///
/// [`KeyBinding::new`](crate::KeyBinding::new) uses it, so bindings created in code always use
/// the keys as written.
pub struct DummyKeyboardMapper;

impl PlatformKeyboardMapper for DummyKeyboardMapper {
    fn map_key_equivalent(
        &self,
        keystroke: Keystroke,
        _use_key_equivalents: bool,
    ) -> KeybindingKeystroke {
        KeybindingKeystroke::from_keystroke(keystroke)
    }

    fn get_key_equivalents(&self) -> Option<&HashMap<char, char>> {
        None
    }
}
