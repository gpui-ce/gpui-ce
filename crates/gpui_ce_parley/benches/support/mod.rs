use gpui::{FontFallbacks, PlatformTextSystem, TextRun, font};
use gpui_ce_parley::{ParleyTextSystem, SystemFonts};
use gpui_fonts::{IBM_PLEX, LILEX, SOURCE_SERIF};
use std::borrow::Cow;

pub fn text_system() -> (ParleyTextSystem, TextRun) {
    let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, IBM_PLEX.family);
    system
        .add_fonts(vec![
            Cow::Borrowed(IBM_PLEX.data),
            Cow::Borrowed(LILEX.data),
            Cow::Borrowed(SOURCE_SERIF.data),
        ])
        .unwrap();
    let mut descriptor = font(IBM_PLEX.family);
    descriptor.fallbacks = Some(FontFallbacks::from_fonts(vec![
        SOURCE_SERIF.family.to_string(),
        LILEX.family.to_string(),
    ]));

    (
        system,
        TextRun {
            len: 0,
            font: descriptor,
            letter_spacing: None,
            ..Default::default()
        },
    )
}
