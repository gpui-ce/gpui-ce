use anyhow::{Result, bail};
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, QueryFamily,
    QueryFont, SourceCache,
};
use gpui::{FontStyle as GpuiFontStyle, FontWidth as GpuiFontWidth};
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

pub(crate) struct FontCatalog {
    context: FontContext,
}

impl FontCatalog {
    pub(crate) fn new(system_fonts: SystemFonts) -> Self {
        let context = FontContext {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: system_fonts == SystemFonts::Load,
            }),
            source_cache: SourceCache::default(),
        };

        Self { context }
    }

    pub(crate) fn context_mut(&mut self) -> &mut FontContext {
        &mut self.context
    }

    pub(crate) fn register_fonts(&mut self, fonts: &[Blob<u8>]) -> Result<()> {
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
            self.context.collection.register_fonts(blob.clone(), None);
        }

        Ok(())
    }

    /// Returns the available family names in stable display order.
    pub(crate) fn family_names(&mut self) -> Vec<String> {
        let mut names = self
            .context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        names.sort_unstable();
        names.dedup();

        names
    }

    pub(crate) fn resolve_face(
        &mut self,
        families: &[FontFamilyName<'_>],
        width: GpuiFontWidth,
        weight: f32,
        style: GpuiFontStyle,
    ) -> Option<QueryFont> {
        let style = match style {
            GpuiFontStyle::Normal => FontStyle::Normal,
            GpuiFontStyle::Italic => FontStyle::Italic,
            GpuiFontStyle::Oblique => FontStyle::Oblique(None),
        };

        let mut query = self
            .context
            .collection
            .query(&mut self.context.source_cache);

        query.set_families(families.iter().map(|family| match family {
            FontFamilyName::Named(name) => QueryFamily::Named(name.as_ref()),
            FontFamilyName::Generic(generic) => QueryFamily::Generic(*generic),
        }));

        query.set_attributes(Attributes::new(
            FontWidth::from_percentage(width.0),
            style,
            FontWeight::new(weight),
        ));

        query.first_available_font()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_fonts::{IBM_PLEX, IBM_PLEX_SEMIBOLD_ITALIC, LILEX, WIDTH_CONDENSED, WIDTH_REGULAR};

    fn register_bytes(catalog: &mut FontCatalog, fonts: &[&[u8]]) -> Result<()> {
        let blobs = fonts
            .iter()
            .map(|bytes| Blob::from(bytes.to_vec()))
            .collect::<Vec<_>>();

        catalog.register_fonts(&blobs)
    }

    #[test]
    fn registered_fonts_are_enumerated_and_resolved() {
        let mut catalog = FontCatalog::new(SystemFonts::Skip);

        register_bytes(
            &mut catalog,
            &[
                IBM_PLEX.data,
                IBM_PLEX_SEMIBOLD_ITALIC.data,
                LILEX.data,
                WIDTH_REGULAR.data,
                WIDTH_CONDENSED.data,
            ],
        )
        .unwrap();

        assert_eq!(
            catalog.family_names(),
            [WIDTH_REGULAR.family, IBM_PLEX.family, LILEX.family]
        );

        let families = [FontFamilyName::Named(IBM_PLEX.family.into())];

        let mut resolve =
            |weight, style| catalog.resolve_face(&families, GpuiFontWidth::NORMAL, weight, style);

        let latin = resolve(400.0, GpuiFontStyle::Normal).unwrap();
        assert_eq!(latin.blob.as_ref(), IBM_PLEX.data);

        assert_eq!(latin.index, 0);

        let semibold_italic = resolve(600.0, GpuiFontStyle::Italic).unwrap();
        assert_eq!(semibold_italic.blob.as_ref(), IBM_PLEX_SEMIBOLD_ITALIC.data);

        let families = [FontFamilyName::Named(WIDTH_REGULAR.family.into())];

        for (width, fixture) in [
            (GpuiFontWidth::NORMAL, WIDTH_REGULAR),
            (GpuiFontWidth::CONDENSED, WIDTH_CONDENSED),
        ] {
            let resolved = catalog
                .resolve_face(&families, width, 400.0, GpuiFontStyle::Normal)
                .unwrap();

            assert_eq!(resolved.blob.as_ref(), fixture.data);
        }
    }

    #[test]
    fn font_registration_is_atomic() {
        let mut catalog = FontCatalog::new(SystemFonts::Skip);
        register_bytes(&mut catalog, &[LILEX.data]).unwrap();
        let families_before = catalog.family_names();

        assert!(register_bytes(&mut catalog, &[IBM_PLEX.data, b"not a font"]).is_err());
        assert_eq!(catalog.family_names(), families_before);

        let families = [FontFamilyName::Named(IBM_PLEX.family.into())];

        assert!(
            catalog
                .resolve_face(
                    &families,
                    GpuiFontWidth::NORMAL,
                    400.0,
                    GpuiFontStyle::Normal,
                )
                .is_none()
        );
    }
}
