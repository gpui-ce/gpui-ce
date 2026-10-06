use std::{borrow::Cow, sync::Arc};

use gpui::PlatformTextSystem;
use gpui_parley::{ParleyTextSystem, SystemFonts};

/// Use a bundled font so headless renders do not depend on installed fonts.
pub fn text_system() -> Arc<dyn PlatformTextSystem> {
    let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
    system
        .add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
        ))])
        .expect("valid bundled font");
    Arc::new(system)
}
