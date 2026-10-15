use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt::{Display, Write},
};

use crate::PlatformKeyboardMapper;

/// A type that can be viewed as a [`Keystroke`].
///
/// Matching code uses this so that it accepts both a [`Keystroke`] that was typed and a
/// [`KeybindingKeystroke`] from a keymap.
pub trait AsKeystroke {
    /// Returns the [`Keystroke`] this value represents.
    fn as_keystroke(&self) -> &Keystroke;
}

/// A key press, with the modifier keys that were held when it happened.
///
/// The platform creates a keystroke for each key press. To write one by hand, as a binding or a
/// test does, parse it from a string with [`Keystroke::parse`]:
///
/// ```
/// # use gpui::Keystroke;
/// let keystroke = Keystroke::parse("ctrl-shift-z").unwrap();
/// assert!(keystroke.modifiers.control);
/// assert!(keystroke.modifiers.shift);
/// assert_eq!(keystroke.key, "z");
/// ```
///
/// See [keyboard input](crate::_documentation::guides::keyboard) for the bigger picture.
#[derive(Clone, Debug, Eq, PartialEq, Default, Deserialize, Hash)]
pub struct Keystroke {
    /// The modifier keys held when the key was pressed.
    pub modifiers: Modifiers,

    /// The key that was pressed, named after the character printed on it, such as `s`, or a
    /// name such as `enter` or `pageup` for keys without a character.
    ///
    /// For option-s on macOS this is `s`, not `ß`. On layouts without ASCII letters, such as Thai,
    /// this is the ASCII equivalent of the key, and the typed character is in `key_char`.
    pub key: String,

    /// The text the key would type with the current layout and modifiers, if any.
    ///
    /// This is `Some("s")` for `s`, `Some("ß")` for option-s on a US macOS layout, and `None` for
    /// `cmd-s`, because `cmd` doesn't type text.
    pub key_char: Option<String>,
}

/// A keystroke as it appears in a keybinding, ready to display to the user.
///
/// A [`KeyBinding`](crate::KeyBinding) stores its keystrokes as `KeybindingKeystroke`s. On
/// Windows, the modifiers and key shown to the user can differ from the ones used for matching.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct KeybindingKeystroke {
    /// The GPUI representation of the keystroke.
    inner: Keystroke,
    /// The modifiers to display.
    #[cfg(target_os = "windows")]
    display_modifiers: Modifiers,
    /// The key to display.
    #[cfg(target_os = "windows")]
    display_key: String,
}

/// Error returned by [`Keystroke::parse`] when a string isn't a valid keystroke.
///
/// It is a plain error type rather than `anyhow::Error`.
#[derive(Debug)]
pub struct InvalidKeystrokeError {
    /// The source text that failed to parse.
    pub keystroke: String,
}

impl Error for InvalidKeystrokeError {}

impl Display for InvalidKeystrokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Invalid keystroke \"{}\". {}",
            self.keystroke, KEYSTROKE_PARSE_EXPECTED_MESSAGE
        )
    }
}

/// The sentence that [`InvalidKeystrokeError`] appends to its message. It describes the syntax
/// [`Keystroke::parse`] expects.
pub const KEYSTROKE_PARSE_EXPECTED_MESSAGE: &str = "Expected a sequence of modifiers \
    (`ctrl`, `alt`, `shift`, `fn`, `cmd`, `super`, or `win`) \
    followed by a key, separated by `-`.";

impl Keystroke {
    /// Returns whether this keystroke, as typed, matches a keystroke in a keymap.
    ///
    /// `self` is the keystroke the user typed and `target` is the one from the binding. Matching
    /// first compares modifiers and key. It also accepts a `target` whose key is this keystroke's
    /// `key_char`, when the target's modifiers are exactly this keystroke's `ctrl` and `cmd` (on
    /// Windows, when the target has no modifiers). That covers layouts where a binding
    /// key sits behind option, such as `$` on a Czech keyboard typed as `alt-ç`, and input methods
    /// that turn a sequence of keys into one character.
    pub fn should_match(&self, target: &KeybindingKeystroke) -> bool {
        #[cfg(not(target_os = "windows"))]
        if let Some(key_char) = self
            .key_char
            .as_ref()
            .filter(|key_char| key_char != &&self.key)
        {
            let ime_modifiers = Modifiers {
                control: self.modifiers.control,
                platform: self.modifiers.platform,
                ..Default::default()
            };

            if &target.inner.key == key_char && target.inner.modifiers == ime_modifiers {
                return true;
            }
        }

        #[cfg(target_os = "windows")]
        if let Some(key_char) = self
            .key_char
            .as_ref()
            .filter(|key_char| key_char != &&self.key)
        {
            // On Windows, if key_char is set, then the typed keystroke produced the key_char
            if &target.inner.key == key_char && target.inner.modifiers == Modifiers::none() {
                return true;
            }
        }

        target.inner.modifiers == self.modifiers && target.inner.key == self.key
    }

    /// Parses a keystroke from text such as `cmd-s`, `ctrl-shift-tab` or `cmd--`.
    ///
    /// The syntax is zero or more modifiers, then a key, separated by `-`:
    ///
    /// - **Modifiers** are `ctrl`, `alt`, `shift`, `fn`, `secondary`, `cmd`, `super` and `win`.
    ///   They can come in any order and any case. `cmd`, `super` and `win` all set
    ///   [`Modifiers::platform`]. `secondary` sets `cmd` on macOS and `ctrl` on other platforms,
    ///   which is what [`Modifiers::secondary`] reports.
    /// - **The key** is the last part. Any other part must be a modifier. Key names are
    ///   case-insensitive and stored in lowercase, so `Tab` and `tab` are the same key. Other
    ///   text is accepted as a key without checking, but the names that platforms produce are
    ///   `space`, `tab`, `enter`, `backspace`, `escape`, `delete`, `insert`, `home`, `end`,
    ///   `pageup`, `pagedown`, `up`, `down`, `left`, `right` and function keys such as `f1`.
    /// - **A single uppercase letter** sets `shift` and uses the lowercase letter as the key, so
    ///   `S` is `shift-s`.
    /// - **A key of `-`** is written as an extra dash, so `cmd--` is `cmd` and `-`.
    /// - **A modifier with no key** becomes the key. The first of `shift`, `ctrl`, `alt`, `cmd` and
    ///   `fn` that is present is used, in that order. Its key is named `shift`, `control`, `alt`,
    ///   `platform` or `function`, and that modifier is cleared. So `ctrl` alone is the key
    ///   `control` with no modifiers set.
    /// - **`key->key_char`** sets `key_char` to the text after `->`, for example `alt-s->ß`.
    ///   [`Keystroke::should_match`] ignores it when matching a binding.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidKeystrokeError`] if a part before the key isn't a modifier, as in
    /// `ctrl-a-b`, or if `->` isn't followed by a key character.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Keystroke;
    /// let save = Keystroke::parse("secondary-s").unwrap();
    /// assert!(save.modifiers.secondary());
    /// assert_eq!(save.key, "s");
    ///
    /// let shifted = Keystroke::parse("S").unwrap();
    /// assert!(shifted.modifiers.shift);
    /// assert_eq!(shifted.key, "s");
    ///
    /// assert!(Keystroke::parse("ctrl-a-b").is_err());
    /// ```
    ///
    /// See [keyboard input](crate::_documentation::guides::keyboard) for how bindings use these.
    pub fn parse(source: &str) -> std::result::Result<Self, InvalidKeystrokeError> {
        let mut modifiers = Modifiers::none();
        let mut key = None;
        let mut key_char = None;

        let mut components = source.split('-').peekable();
        while let Some(component) = components.next() {
            if component.eq_ignore_ascii_case("ctrl") {
                modifiers.control = true;
                continue;
            }
            if component.eq_ignore_ascii_case("alt") {
                modifiers.alt = true;
                continue;
            }
            if component.eq_ignore_ascii_case("shift") {
                modifiers.shift = true;
                continue;
            }
            if component.eq_ignore_ascii_case("fn") {
                modifiers.function = true;
                continue;
            }
            if component.eq_ignore_ascii_case("secondary") {
                if cfg!(target_os = "macos") {
                    modifiers.platform = true;
                } else {
                    modifiers.control = true;
                };
                continue;
            }

            let is_platform = component.eq_ignore_ascii_case("cmd")
                || component.eq_ignore_ascii_case("super")
                || component.eq_ignore_ascii_case("win");

            if is_platform {
                modifiers.platform = true;
                continue;
            }

            let mut key_str = component.to_string();

            if let Some(next) = components.peek() {
                if next.is_empty() && source.ends_with('-') {
                    key = Some(String::from("-"));
                    break;
                } else if next.len() > 1 && next.starts_with('>') {
                    key = Some(key_str);
                    key_char = Some(String::from(&next[1..]));
                    components.next();
                } else {
                    return Err(InvalidKeystrokeError {
                        keystroke: source.to_owned(),
                    });
                }
                continue;
            }

            if component.len() == 1 && component.as_bytes()[0].is_ascii_uppercase() {
                // Convert to shift + lowercase char
                modifiers.shift = true;
                key_str.make_ascii_lowercase();
            } else {
                // convert ascii chars to lowercase so that named keys like "tab" and "enter"
                // are accepted case insensitively and stored how we expect so they are matched properly
                key_str.make_ascii_lowercase()
            }
            key = Some(key_str);
        }

        // Allow for the user to specify a keystroke modifier as the key itself
        // This sets the `key` to the modifier, and disables the modifier
        key = key.or_else(|| {
            use std::mem;
            // std::mem::take clears bool incase its true
            if mem::take(&mut modifiers.shift) {
                Some("shift".to_string())
            } else if mem::take(&mut modifiers.control) {
                Some("control".to_string())
            } else if mem::take(&mut modifiers.alt) {
                Some("alt".to_string())
            } else if mem::take(&mut modifiers.platform) {
                Some("platform".to_string())
            } else if mem::take(&mut modifiers.function) {
                Some("function".to_string())
            } else {
                None
            }
        });

        let key = key.ok_or_else(|| InvalidKeystrokeError {
            keystroke: source.to_owned(),
        })?;

        Ok(Keystroke {
            modifiers,
            key,
            key_char,
        })
    }

    /// Returns text that [`Keystroke::parse`] reads back as this keystroke.
    ///
    /// The platform's name for `cmd` is used: `cmd` on macOS, `super` on Linux and `win` on
    /// Windows. `key_char` is not included, so the result doesn't round-trip when it is set.
    pub fn unparse(&self) -> String {
        unparse(&self.modifiers, &self.key)
    }

    /// Returns whether an input method is still composing text for this keystroke.
    ///
    /// This is true when `key_char` is `None`, the key is printable or empty (not a named key such
    /// as `escape` or `f1`), and none of `cmd`, `ctrl`, `fn` or `alt` is held. `shift` alone
    /// doesn't count.
    pub fn is_ime_in_progress(&self) -> bool {
        self.key_char.is_none()
            && (is_printable_key(&self.key) || self.key.is_empty())
            && !(self.modifiers.platform
                || self.modifiers.control
                || self.modifiers.function
                || self.modifiers.alt)
    }

    /// Returns this keystroke with `key_char` set to the text it would type.
    ///
    /// Simulated keystrokes use this so that they type text as a real key press does. It only
    /// applies when `key_char` is `None` and none of `cmd`, `ctrl`, `fn` or `alt` is held. `space`,
    /// `tab` and `enter` type a space, a tab and a newline. Other printable keys type their
    /// character, uppercased when `shift` is held.
    pub fn with_simulated_ime(mut self) -> Self {
        if self.key_char.is_none()
            && !self.modifiers.platform
            && !self.modifiers.control
            && !self.modifiers.function
            && !self.modifiers.alt
        {
            self.key_char = match self.key.as_str() {
                "space" => Some(" ".into()),
                "tab" => Some("\t".into()),
                "enter" => Some("\n".into()),
                key if !is_printable_key(key) || key.is_empty() => None,
                key => {
                    if self.modifiers.shift {
                        Some(key.to_uppercase())
                    } else {
                        Some(key.into())
                    }
                }
            }
        }
        self
    }
}

impl KeybindingKeystroke {
    #[cfg(target_os = "windows")]
    #[expect(missing_docs)]
    pub fn new(inner: Keystroke, display_modifiers: Modifiers, display_key: String) -> Self {
        KeybindingKeystroke {
            inner,
            display_modifiers,
            display_key,
        }
    }

    /// Creates a keybinding keystroke from `inner`, mapped by `keyboard_mapper`.
    ///
    /// The platform's [`PlatformKeyboardMapper`](crate::PlatformKeyboardMapper) decides what
    /// changes. On macOS, when `use_key_equivalents` is true and the current layout types a
    /// different character for the key, the key is replaced with that character.
    pub fn new_with_mapper(
        inner: Keystroke,
        use_key_equivalents: bool,
        keyboard_mapper: &dyn PlatformKeyboardMapper,
    ) -> Self {
        keyboard_mapper.map_key_equivalent(inner, use_key_equivalents)
    }

    /// Creates a keybinding keystroke from `keystroke` with no platform-specific mapping.
    pub fn from_keystroke(keystroke: Keystroke) -> Self {
        #[cfg(target_os = "windows")]
        {
            let key = keystroke.key.clone();
            let modifiers = keystroke.modifiers;
            KeybindingKeystroke {
                inner: keystroke,
                display_modifiers: modifiers,
                display_key: key,
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            KeybindingKeystroke { inner: keystroke }
        }
    }

    /// Returns the [`Keystroke`] that matching uses.
    pub fn inner(&self) -> &Keystroke {
        &self.inner
    }

    /// Returns the modifiers.
    ///
    /// Platform-specific behavior:
    /// - On macOS and Linux, this modifiers is the same as `inner.modifiers`, which is the GPUI representation of the keystroke.
    /// - On Windows, this modifiers is the display modifiers, for example, a `ctrl-@` keystroke will have `inner.modifiers` as
    /// `Modifiers::control()` and `display_modifiers` as `Modifiers::control_shift()`.
    pub fn modifiers(&self) -> &Modifiers {
        #[cfg(target_os = "windows")]
        {
            &self.display_modifiers
        }
        #[cfg(not(target_os = "windows"))]
        {
            &self.inner.modifiers
        }
    }

    /// Returns the key.
    ///
    /// Platform-specific behavior:
    /// - On macOS and Linux, this key is the same as `inner.key`, which is the GPUI representation of the keystroke.
    /// - On Windows, this key is the display key, for example, a `ctrl-@` keystroke will have `inner.key` as `@` and `display_key` as `2`.
    pub fn key(&self) -> &str {
        #[cfg(target_os = "windows")]
        {
            &self.display_key
        }
        #[cfg(not(target_os = "windows"))]
        {
            &self.inner.key
        }
    }

    /// Sets the modifiers. On Windows this modifies both `inner.modifiers` and `display_modifiers`.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.inner.modifiers = modifiers;
        #[cfg(target_os = "windows")]
        {
            self.display_modifiers = modifiers;
        }
    }

    /// Sets the key. On Windows this modifies both `inner.key` and `display_key`.
    pub fn set_key(&mut self, key: String) {
        #[cfg(target_os = "windows")]
        {
            self.display_key = key.clone();
        }
        self.inner.key = key;
    }

    /// Returns text that [`Keystroke::parse`] reads back as this keystroke, using the displayed
    /// modifiers and key on Windows. See [`Keystroke::unparse`].
    pub fn unparse(&self) -> String {
        #[cfg(target_os = "windows")]
        {
            unparse(&self.display_modifiers, &self.display_key)
        }
        #[cfg(not(target_os = "windows"))]
        {
            unparse(&self.inner.modifiers, &self.inner.key)
        }
    }

    /// Clears `key_char`, so that matching and display use only `key` and the modifiers.
    pub fn remove_key_char(&mut self) {
        self.inner.key_char = None;
    }
}

fn is_printable_key(key: &str) -> bool {
    !matches!(
        key,
        "f1" | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
            | "f13"
            | "f14"
            | "f15"
            | "f16"
            | "f17"
            | "f18"
            | "f19"
            | "f20"
            | "f21"
            | "f22"
            | "f23"
            | "f24"
            | "f25"
            | "f26"
            | "f27"
            | "f28"
            | "f29"
            | "f30"
            | "f31"
            | "f32"
            | "f33"
            | "f34"
            | "f35"
            | "backspace"
            | "delete"
            | "left"
            | "right"
            | "up"
            | "down"
            | "pageup"
            | "pagedown"
            | "insert"
            | "home"
            | "end"
            | "back"
            | "forward"
            | "escape"
    )
}

impl std::fmt::Display for Keystroke {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_modifiers(&self.modifiers, f)?;
        display_key(&self.key, f)
    }
}

impl std::fmt::Display for KeybindingKeystroke {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_modifiers(self.modifiers(), f)?;
        display_key(self.key(), f)
    }
}

/// The modifier keys held at some point in time.
///
/// Each field is `true` when that key is down. Use [`Modifiers::secondary`] for the shortcut key
/// that matches each platform's convention, and the constructors such as [`Modifiers::shift`] to
/// build a value.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize, Hash, JsonSchema)]
pub struct Modifiers {
    /// The control key.
    #[serde(default)]
    pub control: bool,

    /// The alt key, also known as option on macOS.
    #[serde(default)]
    pub alt: bool,

    /// The shift key.
    #[serde(default)]
    pub shift: bool,

    /// The platform key: command on macOS, the Windows key on Windows, and the super key on Linux.
    #[serde(default)]
    pub platform: bool,

    /// The function key.
    #[serde(default)]
    pub function: bool,
}

impl Modifiers {
    /// Returns whether any modifier key is held.
    pub fn modified(&self) -> bool {
        self.control || self.alt || self.shift || self.platform || self.function
    }

    /// Returns whether the platform's shortcut modifier is held.
    ///
    /// This is `platform` (command) on macOS and `control` on Linux and Windows. Use it for
    /// shortcuts such as save and copy, so that they follow each platform's convention.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Modifiers;
    /// assert!(Modifiers::secondary_key().secondary());
    /// assert!(!Modifiers::shift().secondary());
    /// ```
    pub fn secondary(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.platform
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.control
        }
    }

    /// Returns how many modifier keys are held.
    pub fn number_of_modifiers(&self) -> u8 {
        self.control as u8
            + self.alt as u8
            + self.shift as u8
            + self.platform as u8
            + self.function as u8
    }

    /// Returns [`Modifiers`] with no modifiers held.
    pub fn none() -> Modifiers {
        Default::default()
    }

    /// Returns [`Modifiers`] with only `platform` held: command on macOS, the Windows key on
    /// Windows and the super key on Linux.
    pub fn command() -> Modifiers {
        Modifiers {
            platform: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only the [secondary](Modifiers::secondary) key held: `platform`
    /// on macOS and `control` elsewhere.
    pub fn secondary_key() -> Modifiers {
        #[cfg(target_os = "macos")]
        {
            Modifiers {
                platform: true,
                ..Default::default()
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Modifiers {
                control: true,
                ..Default::default()
            }
        }
    }

    /// Returns [`Modifiers`] with only the Windows key held. This is `platform`, the same as
    /// [`Modifiers::command`].
    pub fn windows() -> Modifiers {
        Modifiers {
            platform: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only the super key held. This is `platform`, the same as
    /// [`Modifiers::command`].
    pub fn super_key() -> Modifiers {
        Modifiers {
            platform: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only control held.
    pub fn control() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only alt held.
    pub fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only shift held.
    pub fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with only the function key held.
    pub fn function() -> Modifiers {
        Modifiers {
            function: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with `platform` and `shift` held.
    pub fn command_shift() -> Modifiers {
        Modifiers {
            shift: true,
            platform: true,
            ..Default::default()
        }
    }

    /// Returns [`Modifiers`] with `control` and `shift` held.
    pub fn control_shift() -> Modifiers {
        Modifiers {
            shift: true,
            control: true,
            ..Default::default()
        }
    }

    /// Returns whether every modifier held in `self` is also held in `other`.
    ///
    /// `Modifiers::none()` is a subset of every value, and no value is a subset of `none()` unless
    /// it is `none()` itself.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Modifiers;
    /// let ctrl_shift = Modifiers::control_shift();
    /// assert!(Modifiers::shift().is_subset_of(&ctrl_shift));
    /// assert!(!Modifiers::alt().is_subset_of(&ctrl_shift));
    /// ```
    pub fn is_subset_of(&self, other: &Modifiers) -> bool {
        (*other & *self) == *self
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;

    fn bitor(mut self, other: Self) -> Self::Output {
        self |= other;
        self
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, other: Self) {
        self.control |= other.control;
        self.alt |= other.alt;
        self.shift |= other.shift;
        self.platform |= other.platform;
        self.function |= other.function;
    }
}

impl std::ops::BitXor for Modifiers {
    type Output = Self;
    fn bitxor(mut self, rhs: Self) -> Self::Output {
        self ^= rhs;
        self
    }
}

impl std::ops::BitXorAssign for Modifiers {
    fn bitxor_assign(&mut self, other: Self) {
        self.control ^= other.control;
        self.alt ^= other.alt;
        self.shift ^= other.shift;
        self.platform ^= other.platform;
        self.function ^= other.function;
    }
}

impl std::ops::BitAnd for Modifiers {
    type Output = Self;
    fn bitand(mut self, rhs: Self) -> Self::Output {
        self &= rhs;
        self
    }
}

impl std::ops::BitAndAssign for Modifiers {
    fn bitand_assign(&mut self, other: Self) {
        self.control &= other.control;
        self.alt &= other.alt;
        self.shift &= other.shift;
        self.platform &= other.platform;
        self.function &= other.function;
    }
}

/// The state of the caps lock key at some point in time.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize, Hash, JsonSchema)]
pub struct Capslock {
    /// Whether caps lock is on.
    #[serde(default)]
    pub on: bool,
}

impl AsKeystroke for Keystroke {
    fn as_keystroke(&self) -> &Keystroke {
        self
    }
}

impl AsKeystroke for KeybindingKeystroke {
    fn as_keystroke(&self) -> &Keystroke {
        &self.inner
    }
}

fn display_modifiers(modifiers: &Modifiers, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if modifiers.control {
        #[cfg(target_os = "macos")]
        f.write_char('^')?;

        #[cfg(not(target_os = "macos"))]
        write!(f, "ctrl-")?;
    }
    if modifiers.alt {
        #[cfg(target_os = "macos")]
        f.write_char('⌥')?;

        #[cfg(not(target_os = "macos"))]
        write!(f, "alt-")?;
    }
    if modifiers.platform {
        #[cfg(target_os = "macos")]
        f.write_char('⌘')?;

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        f.write_char('❖')?;

        #[cfg(target_os = "windows")]
        f.write_char('⊞')?;
    }
    if modifiers.shift {
        #[cfg(target_os = "macos")]
        f.write_char('⇧')?;

        #[cfg(not(target_os = "macos"))]
        write!(f, "shift-")?;
    }
    Ok(())
}

fn display_key(key: &str, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let key = match key {
        #[cfg(target_os = "macos")]
        "backspace" => '⌫',
        #[cfg(target_os = "macos")]
        "up" => '↑',
        #[cfg(target_os = "macos")]
        "down" => '↓',
        #[cfg(target_os = "macos")]
        "left" => '←',
        #[cfg(target_os = "macos")]
        "right" => '→',
        #[cfg(target_os = "macos")]
        "tab" => '⇥',
        #[cfg(target_os = "macos")]
        "escape" => '⎋',
        #[cfg(target_os = "macos")]
        "shift" => '⇧',
        #[cfg(target_os = "macos")]
        "control" => '⌃',
        #[cfg(target_os = "macos")]
        "alt" => '⌥',
        #[cfg(target_os = "macos")]
        "platform" => '⌘',

        key if key.len() == 1 => key.chars().next().unwrap().to_ascii_uppercase(),
        key => return f.write_str(key),
    };
    f.write_char(key)
}

#[inline]
fn unparse(modifiers: &Modifiers, key: &str) -> String {
    let mut result = String::new();
    if modifiers.function {
        result.push_str("fn-");
    }
    if modifiers.control {
        result.push_str("ctrl-");
    }
    if modifiers.alt {
        result.push_str("alt-");
    }
    if modifiers.platform {
        #[cfg(target_os = "macos")]
        result.push_str("cmd-");

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        result.push_str("super-");

        #[cfg(target_os = "windows")]
        result.push_str("win-");
    }
    if modifiers.shift {
        result.push_str("shift-");
    }
    result.push_str(&key);
    result
}
