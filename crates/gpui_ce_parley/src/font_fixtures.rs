pub(crate) struct FontFixture {
    pub(crate) family: &'static str,
    pub(crate) data: &'static [u8],
}

pub(crate) const IBM_PLEX: FontFixture = FontFixture {
    family: "IBM Plex Sans",
    data: include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
};

pub(crate) const LILEX: FontFixture = FontFixture {
    family: "Lilex",
    data: include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf"),
};

pub(crate) const SOURCE_SERIF: FontFixture = FontFixture {
    family: "Source Serif 4",
    data: include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf"),
};

pub(crate) const NOTO_SANS: FontFixture = FontFixture {
    family: "Noto Sans",
    data: include_bytes!("../../../assets/fonts/noto-sans/NotoSans[wdth,wght].subset.ttf"),
};

pub(crate) const NOTO_ARABIC: FontFixture = FontFixture {
    family: "Noto Sans Arabic",
    data: include_bytes!("../../../assets/fonts/noto-sans-arabic/NotoSansArabic-Regular.ttf"),
};

pub(crate) const NOTO_HEBREW: FontFixture = FontFixture {
    family: "Noto Sans Hebrew",
    data: include_bytes!("../../../assets/fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf"),
};

pub(crate) const NOTO_COLOR_EMOJI: FontFixture = FontFixture {
    family: "Noto Color Emoji",
    data: include_bytes!("../../../assets/fonts/noto-color-emoji/NotoColorEmoji.subset.ttf"),
};
