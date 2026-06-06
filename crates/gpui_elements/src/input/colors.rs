use gpui::Hsla;

#[derive(Clone, Copy, Debug)]
pub struct InputColors {
    pub selection: Hsla,
    pub cursor: Hsla,
    pub placeholder: Hsla,
    pub marked: Hsla,
}

impl Default for InputColors {
    fn default() -> Self {
        Self {
            selection: gpui::hsla(0.583, 0.519, 0.31, 1.0),
            cursor: Hsla::white().opacity(0.8),
            marked: Hsla::white().opacity(0.6),
            placeholder: gpui::hsla(0., 0., 0.5, 1.0),
        }
    }
}
