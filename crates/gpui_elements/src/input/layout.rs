#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputLayoutStyle {
    SingleLine,
    MultiLine,
}

impl InputLayoutStyle {
    pub(super) fn sanitize_content<'s>(&self, content: &'s str) -> std::borrow::Cow<'s, str> {
        match self {
            // Strip newlines for single-line input
            Self::SingleLine => {
                std::borrow::Cow::Owned(content.replace('\n', " ").replace('\r', ""))
            }
            Self::MultiLine => std::borrow::Cow::Borrowed(content),
        }
    }

    pub fn axis(&self) -> gpui::Axis {
        match self {
            Self::SingleLine => gpui::Axis::Horizontal,
            Self::MultiLine => gpui::Axis::Vertical,
        }
    }
}
