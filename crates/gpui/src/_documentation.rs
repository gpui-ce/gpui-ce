//! Tutorials, guides and background reading for GPUI applications.
//!
//! These pages accompany the type and method reference. Follow a complete build in the
//! tutorial, or choose a topic below when extending or debugging an existing application.
//!
//! | To… | Read |
//! | --- | --- |
//! | Build an application from its first window | [The tutorial](tutorial) |
//! | Get a specific task done | [The guides](guides) |
//! | Trace state through a frame, or input to a handler | [The concepts](concepts) |
//! | Read a complete program | [The examples](examples) |
//! | Understand a design trade-off | [Design](design) |
//! | Compare GPUI with another toolkit | [Comparison](comparison) |
//! | Check a backend or platform capability | [Platforms](platforms) |
//! | Port code from Zed's GPUI | [Upstream](upstream) |

// An empty module gives each included page a rustdoc scope. Import crate symbols
// there so short links such as [`Entity`] resolve without an explicit crate path.
#![allow(unused_imports)]
use crate::*;

macro_rules! pages {
    ($($name:ident => $path:literal,)*) => {$(
        #[doc = include_str!(concat!("../docs/", $path))]
        pub mod $name {}
    )*};
}

#[doc = include_str!("../docs/tutorial/index.md")]
pub mod tutorial {
    use crate::*;

    pages! {
        ch1_hello_window => "tutorial/1-hello-window.md",
        ch2_state => "tutorial/2-state.md",
        ch3_layout_and_style => "tutorial/3-layout-and-style.md",
        ch4_text_input => "tutorial/4-text-input.md",
        ch5_components => "tutorial/5-components.md",
        ch6_background_work => "tutorial/6-background-work.md",
        ch7_polish => "tutorial/7-polish.md",
        ch8_testing => "tutorial/8-testing.md",
    }
}

#[doc = include_str!("../docs/guides/index.md")]
pub mod guides {
    use crate::*;

    pages! {
        start_a_project => "guides/start-a-project.md",
        keyboard => "guides/keyboard.md",
        focus => "guides/focus.md",
        mouse_and_touch => "guides/mouse-and-touch.md",
        text => "guides/text.md",
        text_input => "guides/text-input.md",
        lists_and_scrolling => "guides/lists-and-scrolling.md",
        sharing_state => "guides/sharing-state.md",
        async_work => "guides/async-work.md",
        animation => "guides/animation.md",
        custom_drawing => "guides/custom-drawing.md",
        images_and_assets => "guides/images-and-assets.md",
        windows => "guides/windows.md",
        platform_services => "guides/platform-services.md",
        accessibility => "guides/accessibility.md",
        testing => "guides/testing.md",
        performance => "guides/performance.md",
        web => "guides/web.md",
        hot_reload => "guides/hot-reload.md",
        troubleshooting => "guides/troubleshooting.md",
    }
}

#[doc = include_str!("../docs/concepts/index.md")]
pub mod concepts {
    use crate::*;

    pages! {
        ownership => "concepts/ownership.md",
        contexts => "concepts/contexts.md",
        the_frame => "concepts/the-frame.md",
        elements => "concepts/elements.md",
        layout_and_style => "concepts/layout-and-style.md",
        input_dispatch => "concepts/input-dispatch.md",
        executors => "concepts/executors.md",
    }
}

pages! {
    design => "design.md",
    comparison => "comparison.md",
    platforms => "platforms.md",
    upstream => "upstream.md",
}

/// Builds a page from an example's summary, run command and complete source.
///
/// The `live` form also emits a `gpui-example` HTML marker. The GPUI site's player
/// uses that marker to run the example in the page; rustdoc alone displays the source.
macro_rules! example {
    ($name:ident, $summary:literal, live) => {
        #[doc = concat!(
            $summary, "\n\n",
            "<div class=\"gpui-example\" data-example=\"", stringify!($name), "\">\n\n",
            "Run it with `cargo run --example ", stringify!($name), "`.\n\n</div>\n\n",
            "```rust\n", include_str!(concat!("../examples/", stringify!($name), ".rs")), "```\n",
        )]
        pub mod $name {}
    };
    ($name:ident, $summary:literal, native) => {
        #[doc = concat!(
            $summary, "\n\n",
            "Run it with `cargo run --example ", stringify!($name), "`. ",
            "It uses desktop platform services, so it doesn't run in a browser.\n\n",
            "```rust\n", include_str!(concat!("../examples/", stringify!($name), ".rs")), "```\n",
        )]
        pub mod $name {}
    };
}

// The examples are compiled as cargo examples, so their pages aren't doctests.
#[doc = include_str!("../docs/examples.md")]
#[cfg(not(doctest))]
pub mod examples {
    example!(
        hello_world,
        "The smallest GPUI program: a window with a styled greeting.",
        live
    );
    example!(
        counter,
        "A click counter connecting entity state, an event handler, and `notify`.",
        live
    );
    example!(
        layout,
        "An application shell built with flexbox and grid.",
        live
    );
    example!(
        styling,
        "Backgrounds, borders, corners, shadows, opacity and blur.",
        live
    );
    example!(
        text,
        "Font sizes, weights, decoration, wrapping, truncation, and styled runs.",
        live
    );
    example!(
        images,
        "Raster and vector images, and how they fit their boxes.",
        live
    );
    example!(
        interaction,
        "Hover, press, clicks, cursors, tooltips, and drag and drop.",
        live
    );
    example!(
        keyboard,
        "Actions, key bindings, key contexts, and focus navigation.",
        live
    );
    example!(text_input, "Single- and multi-line editable text.", live);
    example!(
        list,
        "A virtualized list of ten thousand rows with keyboard selection.",
        live
    );
    example!(
        events,
        "Entities that observe each other and communicate with events.",
        live
    );
    example!(tasks, "A stopwatch and a cancellable background job.", live);
    example!(
        animation,
        "Animations, easing, springs and style transitions.",
        live
    );
    example!(
        canvas,
        "A chart drawn with `canvas` and `PathBuilder`.",
        live
    );
    example!(
        custom_element,
        "A slider written as a custom `Element`.",
        live
    );
    example!(todos, "The finished app from the tutorial.", live);
    example!(
        windows,
        "Opening, placing and closing several windows.",
        native
    );
    example!(notifications, "Sending system notifications.", native);
    example!(
        keyring,
        "Storing credentials in the platform keychain.",
        native
    );
    example!(
        haptics,
        "Haptic feedback on trackpads that support it.",
        native
    );
    example!(
        layer_shell,
        "Wayland layer-shell surfaces, such as panels and overlays.",
        native
    );
}
