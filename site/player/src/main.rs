//! Runs a GPUI example in the browser, such as `play/?example=counter`.
//!
//! The examples live in `crates/gpui/examples` and are compiled into this crate.
//! Their `run` functions open the windows shown in the browser. The same functions
//! run natively through `cargo run --example` and in render tests.

use gpui::App;

macro_rules! examples {
    ($($name:ident,)*) => {
        #[allow(dead_code)]
        #[path = "../../../crates/gpui/examples"]
        mod examples {
            $(pub mod $name;)*
        }

        #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
        fn find(name: &str) -> Option<fn(&mut App)> {
            match name {
                $(stringify!($name) => Some(examples::$name::run),)*
                _ => None,
            }
        }

        #[cfg(test)]
        mod renders {
            $(#[test] fn $name() { super::tests::render(super::examples::$name::run) })*
        }
    };
}

examples! {
    hello_world,
    counter,
    layout,
    styling,
    text,
    images,
    interaction,
    keyboard,
    text_input,
    list,
    events,
    tasks,
    animation,
    canvas,
    custom_element,
    todos,
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    eprintln!("The player runs in a browser. Build it with `just site`.");
}

#[cfg(target_family = "wasm")]
fn main() {
    use std::borrow::Cow;

    gpui_platform::web_init();
    let name = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("example"))
        .unwrap_or_default();
    let run = find(&name).unwrap_or_else(|| panic!("no example named {name:?}"));

    gpui_platform::application().run(move |cx| {
        // Bundle the fonts because the browser platform cannot use the host's font set.
        let fonts: [&'static [u8]; 6] = [
            include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
            include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf"),
            include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"),
            include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf"),
            include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf"),
            include_bytes!("../../../assets/fonts/lilex/Lilex-Bold.ttf"),
        ];
        cx.text_system()
            .add_fonts(fonts.into_iter().map(Cow::Borrowed).collect())
            .expect("the bundled fonts are valid");
        run(cx);
    });
}

#[cfg(test)]
mod tests {
    /// Opens an example headlessly and draws its first frame.
    pub fn render(run: fn(&mut gpui::App)) {
        let mut app = gpui::TestApp::new();
        app.update(run);
        assert!(!app.windows().is_empty(), "the example opened no window");
    }
}
