use anyhow::{Result, bail};
use fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
use parking_lot::RwLock;

#[cfg(test)]
use std::borrow::Cow;

/// Controls whether a Parley text system loads operating-system fonts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemFonts {
    /// Enumerate fonts installed on the operating system.
    #[default]
    Load,
    /// Start with an empty catalog. Applications can still register font data.
    Skip,
}

#[derive(Clone)]
pub(crate) struct CatalogState {
    collection: Collection,
    sources: SourceCache,
    generation: u64,
}

impl CatalogState {
    #[cfg(test)]
    fn register(&mut self, fonts: &[Cow<'static, [u8]>]) -> Result<()> {
        let blobs = fonts
            .iter()
            .map(|bytes| Blob::from(bytes.as_ref().to_vec()))
            .collect::<Vec<_>>();
        self.register_blobs(&blobs)
    }

    pub(crate) fn register_blobs(&mut self, fonts: &[Blob<u8>]) -> Result<()> {
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
            self.collection.register_fonts(blob.clone(), None);
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}

/// Shared font enumeration and registration for GPUI text backends.
pub(crate) struct FontCatalog {
    pub(crate) state: RwLock<CatalogState>,
}

impl FontCatalog {
    /// Creates a font catalog with the selected system-font policy.
    #[cfg(test)]
    fn new(system_fonts: SystemFonts) -> Self {
        let collection = Collection::new(CollectionOptions {
            shared: true,
            system_fonts: system_fonts == SystemFonts::Load,
        });
        Self {
            state: RwLock::new(CatalogState {
                collection,
                sources: SourceCache::new_shared(),
                generation: 0,
            }),
        }
    }

    pub(crate) fn from_shared(collection: Collection, sources: SourceCache) -> Self {
        Self {
            state: RwLock::new(CatalogState {
                collection,
                sources,
                generation: 0,
            }),
        }
    }

    /// Registers every face found in the supplied font data.
    #[cfg(test)]
    fn register_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let mut state = self.state.write();
        let mut next = state.clone();
        next.register(&fonts)?;
        *state = next;
        Ok(())
    }

    /// Returns the available family names in stable display order.
    pub(crate) fn family_names(&self) -> Vec<String> {
        let mut state = self.state.write();
        let mut names = state
            .collection
            .family_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Returns the number of successful registration batches.
    pub(crate) fn generation(&self) -> u64 {
        self.state.read().generation
    }

    /// Returns a loaded face matching the requested family and attributes.
    pub(crate) fn resolve(&self, request: &FaceRequest<'_>) -> Option<ResolvedFace> {
        resolve(&mut self.state.write(), request)
    }
}

/// Font attributes understood by the shared catalog.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FaceRequest<'a> {
    /// Ordered font families to query.
    pub(crate) families: &'a [FaceFamily<'a>],
    /// OpenType weight, normally in the range 1 through 1000.
    pub(crate) weight: f32,
    /// Requested style.
    pub(crate) style: gpui::FontStyle,
    /// Optional character that the selected face must cover.
    pub(crate) character: Option<char>,
}

/// A named or generic family used for direct font resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FaceFamily<'a> {
    /// A concrete family name.
    Named(&'a str),
    /// The platform's user-interface font.
    SystemUi,
}

/// A font face selected by Fontique.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedFace {
    /// Raw font data.
    pub(crate) data: Blob<u8>,
    /// Face index within a font collection.
    pub(crate) index: u32,
    /// Synthetic styling recommended by Fontique.
    pub(crate) synthesis: fontique::Synthesis,
}

fn resolve(state: &mut CatalogState, request: &FaceRequest<'_>) -> Option<ResolvedFace> {
    use fontique::{Attributes, FontStyle, FontWeight, FontWidth, QueryFamily, QueryStatus};

    let style = match request.style {
        gpui::FontStyle::Normal => FontStyle::Normal,
        gpui::FontStyle::Italic => FontStyle::Italic,
        gpui::FontStyle::Oblique => FontStyle::Oblique(None),
    };
    let mut selected = None;
    {
        let mut query = state.collection.query(&mut state.sources);
        query.set_families(request.families.iter().map(|family| match family {
            FaceFamily::Named(name) => QueryFamily::Named(name),
            FaceFamily::SystemUi => QueryFamily::Generic(GenericFamily::SystemUi),
        }));
        query.set_attributes(Attributes::new(
            FontWidth::NORMAL,
            style,
            FontWeight::new(request.weight),
        ));
        query.matches_with(|font| {
            if request.character.is_some_and(|character| {
                font.charmap()
                    .and_then(|charmap| charmap.map(character))
                    .is_none()
            }) {
                return QueryStatus::Continue;
            }
            selected = Some((font.blob.clone(), font.index, font.synthesis));
            QueryStatus::Stop
        });
    }
    selected.map(|(data, index, synthesis)| ResolvedFace {
        data,
        index,
        synthesis,
    })
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
        let catalog = FontCatalog::new(SystemFonts::Skip);
        catalog
            .register_fonts(vec![
                Cow::Borrowed(IBM_PLEX),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD_ITALIC),
                Cow::Borrowed(LILEX),
            ])
            .unwrap();

        assert_eq!(catalog.family_names(), ["IBM Plex Sans", "Lilex"]);

        let latin = catalog
            .resolve(&FaceRequest {
                families: &[FaceFamily::Named("IBM Plex Sans")],
                weight: 400.0,
                style: gpui::FontStyle::Normal,
                character: Some('m'),
            })
            .unwrap();
        assert_eq!(latin.data.as_ref(), IBM_PLEX);
        assert_eq!(latin.index, 0);

        let semibold_italic = catalog
            .resolve(&FaceRequest {
                families: &[FaceFamily::Named("IBM Plex Sans")],
                weight: 600.0,
                style: gpui::FontStyle::Italic,
                character: None,
            })
            .unwrap();
        assert_eq!(semibold_italic.data.as_ref(), IBM_PLEX_SEMIBOLD_ITALIC);

        assert!(
            catalog
                .resolve(&FaceRequest {
                    families: &[FaceFamily::Named("IBM Plex Sans")],
                    weight: 400.0,
                    style: gpui::FontStyle::Normal,
                    character: Some('\u{1F9A5}'),
                })
                .is_none()
        );
    }

    #[test]
    fn font_registration_is_atomic() {
        let catalog = FontCatalog::new(SystemFonts::Skip);
        catalog.register_fonts(vec![Cow::Borrowed(LILEX)]).unwrap();
        let families_before = catalog.family_names();

        assert!(
            catalog
                .register_fonts(vec![Cow::Borrowed(IBM_PLEX), Cow::Borrowed(b"not a font")])
                .is_err()
        );
        assert_eq!(catalog.family_names(), families_before);
        assert!(
            catalog
                .resolve(&FaceRequest {
                    families: &[FaceFamily::Named("IBM Plex Sans")],
                    weight: 400.0,
                    style: gpui::FontStyle::Normal,
                    character: None,
                })
                .is_none()
        );
    }
}
