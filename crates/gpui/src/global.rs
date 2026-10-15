use crate::{App, BorrowAppContext};

/// Marks a type whose value is stored once per app.
///
/// Implement it for a type, then store one value of that type with [`App::set_global`]. Read it with
/// [`App::global`], and change it with [`UpdateGlobal::update_global`]. Use a global for state that
/// exists once for the whole app, such as settings or a theme. Use an entity for state that can have
/// several instances, or that several parts of the app change independently.
///
/// The trait is a marker: it adds no methods of its own. Helper traits with blanket implementations,
/// such as [`ReadGlobal`] and [`UpdateGlobal`], add methods to every `Global` type.
///
/// # Restricting access to globals
///
/// In some situations you may need to store some global state, but want to
/// restrict access to reading it or writing to it.
///
/// In these cases, Rust's visibility system can be used to restrict access to
/// a global value. For example, you can create a private struct that implements
/// [`Global`] and holds the global state. Then create a newtype struct that wraps
/// the global type and create custom accessor methods to expose the desired subset
/// of operations.
///
/// # Examples
///
/// ```
/// # use gpui::{Global, TestApp};
/// struct Theme {
///     dark: bool,
/// }
///
/// impl Global for Theme {}
///
/// # let mut app = TestApp::new();
/// app.set_global(Theme { dark: false });
/// assert!(!app.read(|cx| cx.global::<Theme>().dark));
/// ```
pub trait Global: 'static {
    // This trait is intentionally left empty, by virtue of being a marker trait.
    //
    // Use additional traits with blanket implementations to attach functionality
    // to types that implement `Global`.
}

/// Reads the global of the implementing type from an [`App`], as in `Theme::global(cx)`.
///
/// Every [`Global`] type implements this trait. Use it to name a global by type instead of calling
/// [`App::global`] with a turbofish.
///
/// # Examples
///
/// ```
/// # use gpui::{Global, ReadGlobal, TestApp};
/// struct Theme {
///     dark: bool,
/// }
///
/// impl Global for Theme {}
///
/// # let mut app = TestApp::new();
/// app.set_global(Theme { dark: true });
/// assert!(app.read(|cx| Theme::global(cx).dark));
/// ```
pub trait ReadGlobal {
    /// Returns the global of this type.
    ///
    /// # Panics
    ///
    /// Panics if no global of this type has been set.
    fn global(cx: &App) -> &Self;
}

impl<T: Global> ReadGlobal for T {
    fn global(cx: &App) -> &Self {
        cx.global::<T>()
    }
}

/// Changes or sets the global of the implementing type through a context.
///
/// Every [`Global`] type implements this trait. `Theme::update_global(cx, ...)` is the same as
/// `cx.update_global::<Theme, _>(...)`, and `Theme::set_global(cx, ...)` is the same as
/// `cx.set_global(...)`. Both accept any context that implements `BorrowAppContext`, such as
/// `&mut App`.
///
/// # Examples
///
/// ```
/// # use gpui::{Global, TestApp, UpdateGlobal};
/// struct Theme {
///     dark: bool,
/// }
///
/// impl Global for Theme {}
///
/// # let mut app = TestApp::new();
/// app.update(|cx| {
///     Theme::set_global(cx, Theme { dark: false });
///     Theme::update_global(cx, |theme, _cx| theme.dark = true);
/// });
/// assert!(app.read(|cx| cx.global::<Theme>().dark));
/// ```
pub trait UpdateGlobal {
    /// Runs `update` with mutable access to the global and context.
    ///
    /// The global is taken out of the app while `update` runs, so `update` can't read it through the
    /// context. Observers of the global are notified when the update finishes.
    ///
    /// # Panics
    ///
    /// Panics if no global of this type has been set, or if this global is already being updated.
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R;

    /// Sets the global of this type to `global`, replacing any previous value.
    ///
    /// Observers of the global are notified when the update finishes.
    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext;
}

impl<T: Global> UpdateGlobal for T {
    #[track_caller]
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R,
    {
        cx.update_global(update)
    }

    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext,
    {
        cx.set_global(global)
    }
}
