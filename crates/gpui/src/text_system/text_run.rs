use crate::{Font, Pixels, StrikethroughStyle, UnderlineStyle};
use palette::Hsla;
use std::{
    fmt::Debug,
    hash::{Hash, Hasher},
};

/// A stretch of text that shares one font and style.
///
/// Text layout takes a slice of runs whose `len` values add up to the length of
/// the text. Each run styles the bytes that follow the runs before it. Runs
/// control how text is shaped and painted, so changing a run's `color`,
/// `background_color`, `underline`, or `strikethrough` does not reshape the text.
///
/// # Examples
///
/// ```
/// # use gpui::{TextRun, font, hsla};
/// let text = "Hello world";
/// let runs = [
///     TextRun {
///         len: 6,
///         font: font("Lilex"),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     },
///     TextRun {
///         len: 5,
///         font: font("Lilex").bold(),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     },
/// ];
/// assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
/// ```
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextRun {
    /// The number of UTF-8 bytes this run covers.
    pub len: usize,
    /// The font to use for this run.
    pub font: Font,
    /// The color of the glyphs.
    pub color: Hsla,
    /// The background color behind the glyphs, or `None` for no background.
    pub background_color: Option<Hsla>,
    /// The underline drawn under the glyphs, or `None` for no underline.
    pub underline: Option<UnderlineStyle>,
    /// The strikethrough drawn through the glyphs, or `None` for none.
    pub strikethrough: Option<StrikethroughStyle>,
    /// The extra space added between glyphs, in logical pixels, or `None` for no extra space.
    pub letter_spacing: Option<Pixels>,
}

impl Eq for TextRun {}

impl Hash for TextRun {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_float<H: Hasher>(value: f32, state: &mut H) {
            if value == 0.0 {
                0.0f32.to_bits().hash(state);
            } else {
                value.to_bits().hash(state);
            }
        }

        fn hash_color<H: Hasher>(color: Hsla, state: &mut H) {
            let color = crate::hsla_to_rgba(color);
            hash_float(color.color.red, state);
            hash_float(color.color.green, state);
            hash_float(color.color.blue, state);
            hash_float(color.alpha, state);
        }

        self.len.hash(state);
        self.font.hash(state);
        hash_color(self.color, state);
        if let Some(color) = self.background_color {
            hash_color(color, state);
        }
        self.background_color.is_some().hash(state);
        self.underline.is_some().hash(state);
        if let Some(underline) = self.underline {
            hash_float(underline.thickness.into(), state);
            underline.color.is_some().hash(state);
            if let Some(color) = underline.color {
                hash_color(color, state);
            }
            underline.wavy.hash(state);
        }
        self.strikethrough.is_some().hash(state);
        if let Some(strikethrough) = self.strikethrough {
            hash_float(strikethrough.thickness.into(), state);
            strikethrough.color.is_some().hash(state);
            if let Some(color) = strikethrough.color {
                hash_color(color, state);
            }
        }
        self.letter_spacing.is_some().hash(state);
        if let Some(letter_spacing) = self.letter_spacing {
            hash_float(letter_spacing.into(), state);
        }
    }
}
