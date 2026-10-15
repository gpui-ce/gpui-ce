use crate::{Action, App, Platform, SharedString};

/// A menu in the application's menu bar, or a submenu inside another menu.
///
/// Build a menu with [`Menu::new`] and fill it with [`Menu::items`]. Pass the menus to
/// [`App::set_menus`] to show them. Each call to `set_menus` replaces the whole menu bar.
/// Only macOS draws the menu bar from these menus. Linux and Windows store them, and
/// [`App::get_menus`] returns them, but they don't draw a menu bar.
///
/// Picking an [`MenuItem::Action`] dispatches its action through [`App::dispatch_action`].
/// An [`App::on_action`] handler for that action runs for the menu item and for its key
/// binding alike.
///
/// # Examples
///
/// ```standalone_crate
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType, TestApp, actions};
///
/// actions!(app_menu_example, [Preferences, Undo]);
///
/// fn set_up_menus(cx: &mut App, preferences_opened: Rc<Cell<bool>>) {
///     // A menu item and a key binding for the same action share one handler.
///     cx.on_action(move |_: &Preferences, _cx| preferences_opened.set(true));
///     cx.on_action(|_: &Undo, _cx| {
///         // Undo the last edit here.
///     });
///     cx.set_menus([
///         Menu::new("App").items([
///             MenuItem::os_submenu("Services", SystemMenuType::Services),
///             MenuItem::separator(),
///             MenuItem::action("Preferences...", Preferences),
///         ]),
///         Menu::new("Edit").items([MenuItem::os_action("Undo", Undo, OsAction::Undo)]),
///     ]);
/// }
///
/// # let opened = Rc::new(Cell::new(false));
/// # let mut app = TestApp::new();
/// # app.update(|cx| set_up_menus(cx, opened.clone()));
/// // Picking the menu item dispatches its action. The test harness has no menu bar,
/// // so it dispatches the action directly.
/// # app.update(|cx| cx.dispatch_action(&Preferences));
/// # assert!(opened.get());
/// ```
pub struct Menu {
    /// The title of the menu, as shown in the menu bar.
    pub name: SharedString,

    /// The entries of the menu, in display order.
    pub items: Vec<MenuItem>,

    /// Whether the menu is disabled. On macOS, a disabled submenu can't be opened.
    pub disabled: bool,
}

impl Menu {
    /// Creates an empty menu with the given title.
    ///
    /// Add entries with [`Self::items`].
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            items: vec![],
            disabled: false,
        }
    }

    /// Replaces the menu's entries with `items`, and returns the menu.
    ///
    /// The previous entries are discarded.
    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    /// Sets whether the menu is disabled, and returns the menu.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Converts this menu into an [`OwnedMenu`], the form that [`App::get_menus`] returns.
    ///
    /// Platforms call this when they store the menu bar set with [`App::set_menus`], so you
    /// rarely need it.
    pub fn owned(self) -> OwnedMenu {
        OwnedMenu {
            name: self.name.to_string().into(),
            items: self.items.into_iter().map(|item| item.owned()).collect(),
            disabled: self.disabled,
        }
    }
}

/// A submenu that the operating system fills in, such as the Services menu on macOS.
///
/// Create one with [`MenuItem::os_submenu`]. The operating system decides what the menu
/// contains.
pub struct OsMenu {
    /// The title of the menu.
    pub name: SharedString,

    /// Which system menu to show.
    pub menu_type: SystemMenuType,
}

impl OsMenu {
    /// Converts this menu into an [`OwnedOsMenu`].
    pub fn owned(self) -> OwnedOsMenu {
        OwnedOsMenu {
            name: self.name.to_string().into(),
            menu_type: self.menu_type,
        }
    }
}

/// The kinds of menu that the operating system fills in.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum SystemMenuType {
    /// The Services menu in the application menu on macOS, which lists the services that
    /// other apps provide.
    Services,
}

/// An entry in a [`Menu`]: a separator, a submenu, a system menu, or an action.
///
/// Build entries with the constructors on this type, such as [`MenuItem::action`] and
/// [`MenuItem::separator`].
pub enum MenuItem {
    /// A horizontal line that separates groups of entries.
    Separator,

    /// A menu nested inside another menu.
    Submenu(Menu),

    /// A submenu that the operating system fills in, such as the Services menu on macOS.
    SystemMenu(OsMenu),

    /// An entry that dispatches an action when the user picks it.
    Action {
        /// The label of the entry.
        name: SharedString,

        /// The action dispatched when the entry is picked.
        action: Box<dyn Action>,

        /// The standard command this entry stands for, if any. See [`OsAction`].
        os_action: Option<OsAction>,

        /// Whether the entry shows a checkmark. On macOS, the checkmark is shown when this is
        /// `true`.
        checked: bool,

        /// Whether the entry is disabled. On macOS, a disabled entry is greyed out and can't be
        /// picked.
        disabled: bool,
    },
}

impl MenuItem {
    /// Returns a separator entry.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Returns an entry that opens `menu` as a submenu.
    pub fn submenu(menu: Menu) -> Self {
        Self::Submenu(menu)
    }

    /// Returns a submenu titled `name` that the operating system fills in.
    ///
    /// Use it for menus like the macOS Services menu, which the system builds itself.
    pub fn os_submenu(name: impl Into<SharedString>, menu_type: SystemMenuType) -> Self {
        Self::SystemMenu(OsMenu {
            name: name.into(),
            menu_type,
        })
    }

    /// Returns an entry titled `name` that dispatches `action` when the user picks it.
    ///
    /// Use [`Self::os_action`] instead when the entry is a standard command, such as Copy
    /// or Undo.
    pub fn action(name: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Action {
            name: name.into(),
            action: Box::new(action),
            os_action: None,
            checked: false,
            disabled: false,
        }
    }

    /// Returns an entry like [`Self::action`], tagged with the standard command it stands for.
    ///
    /// On macOS, Cut, Copy, Paste and Select All are sent to the responder chain, so the
    /// focused text view handles them. Undo, Redo and entries without an `OsAction` dispatch
    /// `action` as usual.
    pub fn os_action(
        name: impl Into<SharedString>,
        action: impl Action,
        os_action: OsAction,
    ) -> Self {
        Self::Action {
            name: name.into(),
            action: Box::new(action),
            os_action: Some(os_action),
            checked: false,
            disabled: false,
        }
    }

    /// Converts this entry into an [`OwnedMenuItem`].
    pub fn owned(self) -> OwnedMenuItem {
        match self {
            MenuItem::Separator => OwnedMenuItem::Separator,
            MenuItem::Submenu(submenu) => OwnedMenuItem::Submenu(submenu.owned()),
            MenuItem::Action {
                name,
                action,
                os_action,
                checked,
                disabled,
            } => OwnedMenuItem::Action {
                name: name.into(),
                action,
                os_action,
                checked,
                disabled,
            },
            MenuItem::SystemMenu(os_menu) => OwnedMenuItem::SystemMenu(os_menu.owned()),
        }
    }

    /// Sets whether the entry shows a checkmark, and returns the entry.
    ///
    /// Has an effect only on [`MenuItem::Action`]. Other kinds of entry ignore it.
    pub fn checked(mut self, checked: bool) -> Self {
        match &mut self {
            MenuItem::Action { checked: old, .. } => {
                *old = checked;
            }
            _ => {}
        }
        self
    }

    /// Returns whether the entry shows a checkmark.
    ///
    /// Returns `false` for every kind of entry except [`MenuItem::Action`].
    #[inline]
    pub fn is_checked(&self) -> bool {
        match self {
            MenuItem::Action { checked, .. } => *checked,
            _ => false,
        }
    }

    /// Sets whether the entry is disabled, and returns the entry.
    ///
    /// Has an effect on [`MenuItem::Action`] and [`MenuItem::Submenu`]. Other kinds of entry
    /// ignore it.
    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            MenuItem::Action { disabled: old, .. } => {
                *old = disabled;
            }
            MenuItem::Submenu(submenu) => {
                submenu.disabled = disabled;
            }
            _ => {}
        }
        self
    }

    /// Returns whether the entry is disabled.
    ///
    /// Returns `false` for separators and system menus.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        match self {
            MenuItem::Action { disabled, .. } => *disabled,
            MenuItem::Submenu(submenu) => submenu.disabled,
            _ => false,
        }
    }
}

/// An [`OsMenu`] in the form that [`App::get_menus`] returns.
#[derive(Clone)]
pub struct OwnedOsMenu {
    /// The title of the menu.
    pub name: SharedString,

    /// Which system menu to show.
    pub menu_type: SystemMenuType,
}

/// A [`Menu`] in the form that [`App::get_menus`] returns.
///
/// Its entries are [`OwnedMenuItem`]s. The data is the same as the [`Menu`] that was set.
#[derive(Clone)]
pub struct OwnedMenu {
    /// The title of the menu, as shown in the menu bar.
    pub name: SharedString,

    /// The entries of the menu, in display order.
    pub items: Vec<OwnedMenuItem>,

    /// Whether the menu is disabled.
    pub disabled: bool,
}

/// A [`MenuItem`] in the form that [`App::get_menus`] returns.
pub enum OwnedMenuItem {
    /// A horizontal line that separates groups of entries.
    Separator,

    /// A menu nested inside another menu.
    Submenu(OwnedMenu),

    /// A submenu that the operating system fills in.
    SystemMenu(OwnedOsMenu),

    /// An entry that dispatches an action when the user picks it.
    Action {
        /// The label of the entry.
        name: String,

        /// The action dispatched when the entry is picked.
        action: Box<dyn Action>,

        /// The standard command this entry stands for, if any. See [`OsAction`].
        os_action: Option<OsAction>,

        /// Whether the entry shows a checkmark.
        checked: bool,

        /// Whether the entry is disabled.
        disabled: bool,
    },
}

impl Clone for OwnedMenuItem {
    fn clone(&self) -> Self {
        match self {
            OwnedMenuItem::Separator => OwnedMenuItem::Separator,
            OwnedMenuItem::Submenu(submenu) => OwnedMenuItem::Submenu(submenu.clone()),
            OwnedMenuItem::Action {
                name,
                action,
                os_action,
                checked,
                disabled,
            } => OwnedMenuItem::Action {
                name: name.clone(),
                action: action.boxed_clone(),
                os_action: *os_action,
                checked: *checked,
                disabled: *disabled,
            },
            OwnedMenuItem::SystemMenu(os_menu) => OwnedMenuItem::SystemMenu(os_menu.clone()),
        }
    }
}

// TODO: As part of the global selections refactor, these should
// be moved to GPUI-provided actions that make this association
// without leaking the platform details to GPUI users

/// A standard command that the operating system knows about, such as Copy or Undo.
///
/// Tag a menu entry with one of these using [`MenuItem::os_action`], so the operating system can
/// route the command the way it does for other apps. On macOS, the entry's action is still
/// dispatched for Undo and Redo, and the system handles Cut, Copy, Paste and Select All.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum OsAction {
    /// The Cut command.
    Cut,

    /// The Copy command.
    Copy,

    /// The Paste command.
    Paste,

    /// The Select All command.
    SelectAll,

    /// The Undo command.
    Undo,

    /// The Redo command.
    Redo,
}

pub(crate) fn init_app_menus(platform: &dyn Platform, cx: &App) {
    platform.on_will_open_app_menu(Box::new({
        let cx = cx.to_async();
        move || {
            if let Some(app) = cx.app.upgrade() {
                app.borrow_mut().update(|cx| cx.clear_pending_keystrokes());
            }
        }
    }));

    platform.on_validate_app_menu_command(Box::new({
        let cx = cx.to_async();
        move |action| {
            cx.app
                .upgrade()
                .map(|app| app.borrow_mut().update(|cx| cx.is_action_available(action)))
                .unwrap_or(false)
        }
    }));

    platform.on_app_menu_action(Box::new({
        let cx = cx.to_async();
        move |action| {
            if let Some(app) = cx.app.upgrade() {
                app.borrow_mut().update(|cx| cx.dispatch_action(action));
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use crate::Menu;

    #[test]
    fn test_menu() {
        let menu = Menu::new("App")
            .items(vec![
                crate::MenuItem::action("Action 1", gpui::NoAction),
                crate::MenuItem::separator(),
            ])
            .disabled(true);

        assert_eq!(menu.name.as_ref(), "App");
        assert_eq!(menu.items.len(), 2);
        assert!(menu.disabled);
    }

    #[test]
    fn test_menu_item_builder() {
        use super::MenuItem;

        let item = MenuItem::action("Test Action", gpui::NoAction);
        assert_eq!(
            match &item {
                MenuItem::Action { name, .. } => name.as_ref(),
                _ => unreachable!(),
            },
            "Test Action"
        );
        assert!(matches!(
            item,
            MenuItem::Action {
                checked: false,
                disabled: false,
                ..
            }
        ));

        assert!(
            MenuItem::action("Test Action", gpui::NoAction)
                .checked(true)
                .is_checked()
        );
        assert!(
            MenuItem::action("Test Action", gpui::NoAction)
                .disabled(true)
                .is_disabled()
        );

        let submenu = MenuItem::submenu(super::Menu {
            name: "Submenu".into(),
            items: vec![],
            disabled: true,
        });
        assert_eq!(
            match &submenu {
                MenuItem::Submenu(menu) => menu.name.as_ref(),
                _ => unreachable!(),
            },
            "Submenu"
        );
        assert!(!submenu.is_checked());
        assert!(submenu.is_disabled());
    }
}
