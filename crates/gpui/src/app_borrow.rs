//! Module which contains utility traits for various containers around [`App`] which all implement the [`AppContext`](crate::AppContext) subdomain traits.
use crate::{App, AppCell, AsyncApp, AsyncWindowContext, Context};
use std::rc::Rc;

/// Interface to anonymize direct app reference holders (e.g. [`Context`]) and app-ref-counters (e.g. [`AsyncApp`] and `Rc<AppCell>`).
pub trait UpgradeApp {
    /// Returns a struct that can be borrowed from to access an immutable [`App`].
    fn upgrade(&self) -> impl BorrowApp;
    /// Returns a struct that can be borrowed from to access a mutable [`App`].
    fn upgrade_mut(&mut self) -> impl BorrowAppMut;
}
impl UpgradeApp for AsyncApp {
    fn upgrade(&self) -> impl BorrowApp {
        self.app()
    }

    fn upgrade_mut(&mut self) -> impl BorrowAppMut {
        self.app()
    }
}
impl UpgradeApp for AsyncWindowContext {
    fn upgrade(&self) -> impl BorrowApp {
        self.app()
    }

    fn upgrade_mut(&mut self) -> impl BorrowAppMut {
        self.app()
    }
}
impl UpgradeApp for Rc<AppCell> {
    fn upgrade(&self) -> impl BorrowApp {
        self.clone()
    }

    fn upgrade_mut(&mut self) -> impl BorrowAppMut {
        self.clone()
    }
}
impl<T: 'static> UpgradeApp for Context<'_, T> {
    fn upgrade(&self) -> impl BorrowApp {
        self
    }

    fn upgrade_mut(&mut self) -> impl BorrowAppMut {
        self
    }
}

/// Interface to anonymize [`App`] immutable borrows such as [`AppRef`](crate::AppRef), [`&Context`](crate::Context), and [`&App`](crate::App),
pub trait BorrowApp {
    /// Borrows from this type, returning something that can deref into [`App`].
    fn borrow(&self) -> impl std::ops::Deref<Target = App>;
}
/// Interface to anonymize [`App`] mutable borrows such as [`AppRefMut`](crate::AppRefMut), [`&mut Context`](crate::Context), and [`&mut App`](crate::App),
pub trait BorrowAppMut {
    /// Borrows from this type, returning something that can deref mutable into [`App`].
    fn borrow_mut(&mut self) -> impl std::ops::DerefMut<Target = App>;
}
impl BorrowApp for std::rc::Rc<AppCell> {
    fn borrow(&self) -> impl std::ops::Deref<Target = App> {
        AppCell::borrow(self)
    }
}
impl BorrowAppMut for std::rc::Rc<AppCell> {
    fn borrow_mut(&mut self) -> impl std::ops::DerefMut<Target = App> {
        AppCell::borrow_mut(self)
    }
}

impl<T> BorrowApp for &Context<'_, T> {
    fn borrow(&self) -> impl std::ops::Deref<Target = App> {
        <Context<'_, T> as std::ops::Deref>::deref(self)
    }
}
impl<T> BorrowAppMut for &mut Context<'_, T> {
    fn borrow_mut(&mut self) -> impl std::ops::DerefMut<Target = App> {
        <Context<'_, T> as std::ops::DerefMut>::deref_mut(self)
    }
}
