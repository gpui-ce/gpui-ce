use crate::{App, BorrowAppContext};

/// A marker trait for types stored in GPUI global state.
///
/// Implement this on types you want to store as globals. Accessors reject types
/// that do not implement [`Global`] at compile time.
///
/// ## Restricting access to globals
///
/// To limit reads or writes, store state in a private struct implementing [`Global`].
/// Expose a newtype wrapper with accessor methods for only the permitted operations.
pub trait Global: 'static {
    // Attach functionality through additional traits with blanket implementations
    // for types implementing `Global`.
}

/// A trait for reading a global value from the context.
pub trait ReadGlobal {
    /// Returns the global instance of the implementing type.
    ///
    /// Panics if a global for that type has not been assigned.
    fn global(cx: &App) -> &Self;
}

impl<T: Global> ReadGlobal for T {
    fn global(cx: &App) -> &Self {
        cx.global::<T>()
    }
}

/// A trait for updating a global value in the context.
pub trait UpdateGlobal {
    /// Updates the global value through a closure with mutable access to both
    /// the global and the context.
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R;

    /// Set the global instance of the implementing type.
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
