pub(crate) const IBM_PLEX_SEMIBOLD: FontFixture = FontFixture {
    family: IBM_PLEX.family,
    data: include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"),
};

pub(crate) const IBM_PLEX_SEMIBOLD_ITALIC: FontFixture = FontFixture {
    family: IBM_PLEX.family,
    data: include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf"),
};

pub(crate) const WIDTH_REGULAR: FontFixture = FontFixture {
    family: "GPUI Width Fixture",
    data: include_bytes!("../../../assets/fonts/noto-sans/WidthFixture-Regular.ttf"),
};

pub(crate) const WIDTH_CONDENSED: FontFixture = FontFixture {
    family: WIDTH_REGULAR.family,
    data: include_bytes!("../../../assets/fonts/noto-sans/WidthFixture-Condensed.ttf"),
};
