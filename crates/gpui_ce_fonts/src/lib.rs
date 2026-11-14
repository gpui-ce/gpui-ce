pub struct FontFixture {
    pub family: &'static str,
    pub data: &'static [u8],
}

impl std::ops::Deref for FontFixture {
    type Target = &'static [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

pub const IBM_PLEX: FontFixture = FontFixture {
    family: "IBM Plex Sans",
    data: include_bytes!("../fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
};

pub const IBM_PLEX_ITALIC: FontFixture = FontFixture {
    family: "IBM Plex Sans",
    data: include_bytes!("../fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf"),
};

pub const IBM_PLEX_SEMIBOLD: FontFixture = FontFixture {
    family: IBM_PLEX.family,
    data: include_bytes!("../fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"),
};

pub const IBM_PLEX_SEMIBOLD_ITALIC: FontFixture = FontFixture {
    family: IBM_PLEX.family,
    data: include_bytes!("../fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf"),
};

pub const LILEX: FontFixture = FontFixture {
    family: "Lilex",
    data: include_bytes!("../fonts/lilex/Lilex-Regular.ttf"),
};

pub const LILEX_BOLD: FontFixture = FontFixture {
    family: "Lilex",
    data: include_bytes!("../fonts/lilex/Lilex-Bold.ttf"),
};

pub const SOURCE_SERIF: FontFixture = FontFixture {
    family: "Source Serif 4",
    data: include_bytes!("../fonts/source-serif-4/SourceSerif4[opsz-wght].ttf"),
};

pub const NOTO_SANS: FontFixture = FontFixture {
    family: "Noto Sans",
    data: include_bytes!("../fonts/noto-sans/NotoSans[wdth-wght].subset.ttf"),
};

pub const NOTO_ARABIC: FontFixture = FontFixture {
    family: "Noto Sans Arabic",
    data: include_bytes!("../fonts/noto-sans-arabic/NotoSansArabic-Regular.ttf"),
};

pub const NOTO_HEBREW: FontFixture = FontFixture {
    family: "Noto Sans Hebrew",
    data: include_bytes!("../fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf"),
};

pub const NOTO_COLOR_EMOJI: FontFixture = FontFixture {
    family: "Noto Color Emoji",
    data: include_bytes!("../fonts/noto-color-emoji/NotoColorEmoji.subset.ttf"),
};

pub const WIDTH_REGULAR: FontFixture = FontFixture {
    family: "GPUI Width Fixture",
    data: include_bytes!("../fonts/noto-sans/WidthFixture-Regular.ttf"),
};

pub const WIDTH_CONDENSED: FontFixture = FontFixture {
    family: WIDTH_REGULAR.family,
    data: include_bytes!("../fonts/noto-sans/WidthFixture-Condensed.ttf"),
};

/// Builds a TrueType collection from standalone SFNT font fixtures, preserving face order.
///
/// Table offsets are relocated into the collection. Input must be valid standalone fonts.
pub fn test_collection(faces: &[&[u8]]) -> Vec<u8> {
    let mut collection = vec![0; 12 + faces.len() * 4];
    collection[..4].copy_from_slice(b"ttcf");
    collection[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    collection[8..12].copy_from_slice(&(faces.len() as u32).to_be_bytes());

    for (face_idx, face) in faces.iter().enumerate() {
        collection.resize(collection.len().next_multiple_of(4), 0);
        let face_offset = collection.len();
        let offset_position = 12 + face_idx * 4;

        collection[offset_position..offset_position + 4]
            .copy_from_slice(&(face_offset as u32).to_be_bytes());

        collection.extend_from_slice(face);
        let table_count = u16::from_be_bytes(face[4..6].try_into().unwrap()) as usize;

        for table_idx in 0..table_count {
            let offset_position = 12 + table_idx * 16 + 8;

            let table_offset = u32::from_be_bytes(
                face[offset_position..offset_position + 4]
                    .try_into()
                    .unwrap(),
            );

            let collection_offset = table_offset + face_offset as u32;
            let destination = face_offset + offset_position;

            collection[destination..destination + 4]
                .copy_from_slice(&collection_offset.to_be_bytes());
        }
    }

    collection
}
