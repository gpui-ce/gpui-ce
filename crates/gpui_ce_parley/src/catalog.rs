use anyhow::{Result, bail};
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, QueryFamily,
    QueryFont, QueryStatus, SourceCache,
};
use parley::{FontContext, FontFamilyName};

/// Controls whether a Parley text system loads operating-system fonts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemFonts {
    /// Enumerate fonts installed on the operating system.
    #[default]
    Load,
    /// Start with an empty catalog. Applications can still register font data.
    Skip,
}

pub(crate) fn new_font_context(system_fonts: SystemFonts) -> FontContext {
    FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: system_fonts == SystemFonts::Load,
        }),
        source_cache: SourceCache::default(),
    }
}

pub(crate) fn register_font_blobs(context: &mut FontContext, fonts: &[Blob<u8>]) -> Result<()> {
    let mut validator = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });

    for blob in fonts {
        if validator.register_fonts(blob.clone(), None).is_empty() {
            bail!("font data did not contain a supported font face");
        }
    }

    for blob in fonts {
        context.collection.register_fonts(blob.clone(), None);
    }

    Ok(())
}

#[cfg(test)]
fn register_bytes(context: &mut FontContext, fonts: &[&[u8]]) -> Result<()> {
    let blobs = fonts
        .iter()
        .map(|bytes| Blob::from(bytes.to_vec()))
        .collect::<Vec<_>>();

    register_font_blobs(context, &blobs)
}

/// Returns the available family names in stable display order.
pub(crate) fn family_names(context: &mut FontContext) -> Vec<String> {
    let mut names = context
        .collection
        .family_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();

    names
}

pub(crate) fn resolve_face(
    context: &mut FontContext,
    families: &[FontFamilyName<'_>],
    weight: f32,
    style: gpui::FontStyle,
) -> Option<QueryFont> {
    let style = match style {
        gpui::FontStyle::Normal => FontStyle::Normal,
        gpui::FontStyle::Italic => FontStyle::Italic,
        gpui::FontStyle::Oblique => FontStyle::Oblique(None),
    };

    let mut query = context.collection.query(&mut context.source_cache);
    query.set_families(families.iter().map(|family| match family {
        FontFamilyName::Named(name) => QueryFamily::Named(name.as_ref()),
        FontFamilyName::Generic(generic) => QueryFamily::Generic(*generic),
    }));
    query.set_attributes(Attributes::new(
        FontWidth::NORMAL,
        style,
        FontWeight::new(weight),
    ));

    let mut selected = None;
    query.matches_with(|font| {
        selected = Some(font.clone());

        QueryStatus::Stop
    });

    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const IBM_PLEX_SEMIBOLD_ITALIC: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf");
    const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");

    #[test]
    fn registered_fonts_are_enumerated_and_resolved() {
        let mut context = new_font_context(SystemFonts::Skip);
        register_bytes(&mut context, &[IBM_PLEX, IBM_PLEX_SEMIBOLD_ITALIC, LILEX]).unwrap();

        assert_eq!(family_names(&mut context), ["IBM Plex Sans", "Lilex"]);

        let families = [FontFamilyName::Named("IBM Plex Sans".into())];
        let mut resolve = |weight, style| resolve_face(&mut context, &families, weight, style);
        let latin = resolve(400.0, gpui::FontStyle::Normal).unwrap();
        assert_eq!(latin.blob.as_ref(), IBM_PLEX);
        assert_eq!(latin.index, 0);

        let semibold_italic = resolve(600.0, gpui::FontStyle::Italic).unwrap();
        assert_eq!(semibold_italic.blob.as_ref(), IBM_PLEX_SEMIBOLD_ITALIC);
    }

    #[test]
    fn font_registration_is_atomic() {
        let mut context = new_font_context(SystemFonts::Skip);
        register_bytes(&mut context, &[LILEX]).unwrap();
        let families_before = family_names(&mut context);

        assert!(register_bytes(&mut context, &[IBM_PLEX, b"not a font"]).is_err());
        assert_eq!(family_names(&mut context), families_before);

        let families = [FontFamilyName::Named("IBM Plex Sans".into())];
        assert!(resolve_face(&mut context, &families, 400.0, gpui::FontStyle::Normal).is_none());
    }
}
