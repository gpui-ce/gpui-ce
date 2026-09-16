use gpui::{
    Bounds, CaretPosition, Pixels, PlatformTextLayout, Point, Size, TextMovement,
    TextSelectionKind, VisualDirection, point, px,
};
use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation as _;

#[derive(Debug)]
pub(super) struct ParagraphRange {
    pub content: Range<usize>,
    pub separator: Range<usize>,
}

pub(super) fn is_paragraph_separator(character: char) -> bool {
    // Unicode bidi class B. U+2028 forces a line break within the same paragraph.
    matches!(
        character,
        '\n' | '\r' | '\u{001c}'..='\u{001e}' | '\u{0085}' | '\u{2029}'
    )
}

pub(super) fn paragraph_ranges(text: &str) -> Vec<ParagraphRange> {
    let mut paragraphs = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();

    while let Some((idx, character)) = characters.next() {
        if !is_paragraph_separator(character) {
            continue;
        }

        let mut end = idx + character.len_utf8();

        if character == '\r' && characters.peek().is_some_and(|(_idx, next)| *next == '\n') {
            characters.next();
            end += 1;
        }

        paragraphs.push(ParagraphRange {
            content: start..idx,
            separator: idx..end,
        });
        start = end;
    }

    paragraphs.push(ParagraphRange {
        content: start..text.len(),
        separator: text.len()..text.len(),
    });

    paragraphs
}

pub(super) fn local_range(source: &Range<usize>, content: &Range<usize>) -> Option<Range<usize>> {
    let start = source.start.max(content.start);
    let end = source.end.min(content.end);

    if start >= end {
        return None;
    }

    Some(start - content.start..end - content.start)
}

#[derive(Debug)]
pub(super) struct ParagraphLayout {
    pub source: ParagraphRange,
    pub first_line: usize,
    pub block_offset: Pixels,
    pub native: Arc<dyn PlatformTextLayout>,
    pub newline: Range<Pixels>,
    pub is_rtl: bool,
}

impl ParagraphLayout {
    fn local_caret(&self, caret: CaretPosition) -> CaretPosition {
        CaretPosition::new(
            caret
                .index
                .saturating_sub(self.source.content.start)
                .min(self.native.len()),
            caret.affinity,
        )
    }

    fn global_caret(&self, caret: CaretPosition) -> CaretPosition {
        CaretPosition::new(self.source.content.start + caret.index, caret.affinity)
    }

    fn local_point(&self, mut position: Point<Pixels>, line_height: Pixels) -> Point<Pixels> {
        position.y -= line_height * self.first_line;

        position
    }

    fn edge(&self, direction: VisualDirection) -> CaretPosition {
        let position = match direction {
            VisualDirection::Left => point(px(f32::MAX), px(self.native.line_count() as f32 - 0.5)),
            VisualDirection::Right => point(px(-f32::MAX), px(0.5)),
        };
        let caret = self
            .native
            .caret_from_point(position, px(1.0))
            .unwrap_or_else(|caret| caret);

        self.global_caret(caret)
    }
}

#[derive(Debug)]
pub(super) struct ParleyDocumentLayout {
    paragraphs: Vec<ParagraphLayout>,
    size: Size<Pixels>,
    graphemes: Vec<Range<usize>>,
}

impl ParleyDocumentLayout {
    pub fn new(paragraphs: Vec<ParagraphLayout>, text: &str, size: Size<Pixels>) -> Self {
        let graphemes = text
            .grapheme_indices(true)
            .map(|(start, grapheme)| start..start + grapheme.len())
            .collect();

        Self {
            paragraphs,
            size,
            graphemes,
        }
    }

    fn paragraph_for_index(&self, idx: usize) -> usize {
        self.paragraphs
            .partition_point(|paragraph| paragraph.source.content.start <= idx)
            .saturating_sub(1)
    }

    fn paragraph_for_point(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> &ParagraphLayout {
        let line_idx = if line_height > Pixels::ZERO && position.y >= Pixels::ZERO {
            (position.y / line_height) as usize
        } else {
            0
        };
        let paragraph_idx = self
            .paragraphs
            .partition_point(|paragraph| paragraph.first_line <= line_idx)
            .saturating_sub(1);

        &self.paragraphs[paragraph_idx]
    }

    fn adjacent_edge(
        &self,
        paragraph_idx: usize,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let next_idx = match direction {
            VisualDirection::Left => paragraph_idx.checked_sub(1)?,
            VisualDirection::Right => paragraph_idx + 1,
        };

        Some(self.paragraphs.get(next_idx)?.edge(direction))
    }
}

impl PlatformTextLayout for ParleyDocumentLayout {
    fn len(&self) -> usize {
        self.paragraphs.last().unwrap().source.separator.end
    }

    fn line_count(&self) -> usize {
        let paragraph = self.paragraphs.last().unwrap();

        paragraph.first_line + paragraph.native.line_count()
    }

    fn size(&self) -> Size<Pixels> {
        self.size
    }

    fn index_from_point(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        let paragraph = self.paragraph_for_point(position, line_height);

        paragraph
            .native
            .index_from_point(paragraph.local_point(position, line_height), line_height)
            .map(|idx| idx + paragraph.source.content.start)
            .map_err(|idx| idx + paragraph.source.content.start)
    }

    fn caret_from_point(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        let paragraph = self.paragraph_for_point(position, line_height);

        paragraph
            .native
            .caret_from_point(paragraph.local_point(position, line_height), line_height)
            .map(|caret| paragraph.global_caret(caret))
            .map_err(|caret| paragraph.global_caret(caret))
    }

    fn caret_geometry(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        if caret.index > self.len() {
            return None;
        }

        let paragraph = &self.paragraphs[self.paragraph_for_index(caret.index)];
        let mut bounds = paragraph
            .native
            .caret_geometry(paragraph.local_caret(caret), line_height)?;
        bounds.origin.y += line_height * paragraph.first_line;

        Some(bounds)
    }

    fn refresh_caret(&self, caret: CaretPosition) -> CaretPosition {
        let paragraph = &self.paragraphs[self.paragraph_for_index(caret.index)];

        paragraph.global_caret(paragraph.native.refresh_caret(paragraph.local_caret(caret)))
    }

    fn move_visual(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let paragraph_idx = self.paragraph_for_index(caret.index);
        let paragraph = &self.paragraphs[paragraph_idx];

        paragraph
            .native
            .move_visual(paragraph.local_caret(caret), direction)
            .map(|caret| paragraph.global_caret(caret))
            .or_else(|| self.adjacent_edge(paragraph_idx, direction))
    }

    fn selection_geometry(&self, range: Range<usize>, line_height: Pixels) -> Vec<Bounds<Pixels>> {
        let mut regions = Vec::new();

        for paragraph in &self.paragraphs {
            if let Some(local) = local_range(&range, &paragraph.source.content) {
                for mut bounds in paragraph.native.selection_geometry(local, line_height) {
                    bounds.origin.y += line_height * paragraph.first_line;
                    regions.push(bounds);
                }
            }

            if local_range(&range, &paragraph.source.separator).is_none() {
                continue;
            }

            let line_idx = paragraph.first_line + paragraph.native.line_count() - 1;
            regions.push(Bounds::from_corners(
                point(paragraph.newline.start, line_height * line_idx),
                point(paragraph.newline.end, line_height * (line_idx + 1)),
            ));
        }

        regions
    }

    fn inline_geometry(&self, range: Range<usize>) -> Vec<(Bounds<Pixels>, usize)> {
        let mut regions = Vec::new();

        for paragraph in &self.paragraphs {
            let Some(local) = local_range(&range, &paragraph.source.content) else {
                continue;
            };

            for (mut bounds, idx) in paragraph.native.inline_geometry(local) {
                bounds.origin.y += paragraph.block_offset;
                regions.push((bounds, idx + paragraph.first_line));
            }
        }

        regions
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.graphemes
            .iter()
            .rev()
            .find(|range| range.start < caret.index)
            .cloned()
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.graphemes
            .iter()
            .find(|range| range.end > caret.index)
            .cloned()
    }

    fn move_caret(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> (CaretPosition, Option<Pixels>) {
        let caret = self.refresh_caret(caret);
        let direction = match movement {
            TextMovement::VisualLeft | TextMovement::VisualWordLeft => Some(VisualDirection::Left),
            TextMovement::VisualRight | TextMovement::VisualWordRight => {
                Some(VisualDirection::Right)
            }
            _ => None,
        };

        if matches!(
            movement,
            TextMovement::VisualLeft | TextMovement::VisualRight
        ) {
            return (
                self.move_visual(caret, direction.unwrap()).unwrap_or(caret),
                None,
            );
        }

        if matches!(movement, TextMovement::VisualUp | TextMovement::VisualDown) {
            let geometry = self.caret_geometry(caret, px(1.0)).unwrap();
            let delta = if movement == TextMovement::VisualUp {
                -1
            } else {
                1
            };
            let target_idx = (f32::from(geometry.origin.y) as usize)
                .checked_add_signed(delta)
                .filter(|idx| *idx < self.line_count());
            let Some(target_idx) = target_idx else {
                let idx = if delta < 0 { 0 } else { self.len() };

                return (
                    self.refresh_caret(CaretPosition::new(idx, caret.affinity)),
                    preferred_x,
                );
            };
            let x = preferred_x.unwrap_or(geometry.origin.x);
            let moved = self
                .caret_from_point(point(x, px(target_idx as f32 + 0.5)), px(1.0))
                .unwrap_or_else(|caret| caret);

            return (moved, Some(x));
        }

        let paragraph_idx = self.paragraph_for_index(caret.index);
        let paragraph = &self.paragraphs[paragraph_idx];
        let local = paragraph.local_caret(caret);
        let (moved, preferred_x) = paragraph.native.move_caret(local, movement, preferred_x);

        if let Some(direction) = direction
            && paragraph.native.caret_geometry(moved, px(1.0))
                == paragraph.native.caret_geometry(local, px(1.0))
            && let Some(edge) = self.adjacent_edge(paragraph_idx, direction)
        {
            return (edge, preferred_x);
        }

        (paragraph.global_caret(moved), preferred_x)
    }

    fn selection_from_point(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        let paragraph = self.paragraph_for_point(position, line_height);
        let local = paragraph.native.selection_from_point(
            paragraph.local_point(position, line_height),
            line_height,
            kind,
        );
        let start = local.start + paragraph.source.content.start;
        let mut end = local.end + paragraph.source.content.start;

        if matches!(
            kind,
            TextSelectionKind::VisualLine | TextSelectionKind::HardLine
        ) && end == paragraph.source.content.end
        {
            end = paragraph.source.separator.end;
        }

        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_ranges_preserve_separators_empty_paragraphs_and_line_separators() {
        for (text, expected) in [
            ("", vec![(0..0, 0..0)]),
            (
                "אב\r\n\nabc\n",
                vec![
                    (0..4, 4..6),
                    (6..6, 6..7),
                    (7..10, 10..11),
                    (11..11, 11..11),
                ],
            ),
            ("a\u{2028}b\u{2029}c", vec![(0..5, 5..8), (8..9, 9..9)]),
            (
                "\r\u{0085}\u{001c}\u{001d}\u{001e}",
                vec![
                    (0..0, 0..1),
                    (1..1, 1..3),
                    (3..3, 3..4),
                    (4..4, 4..5),
                    (5..5, 5..6),
                    (6..6, 6..6),
                ],
            ),
        ] {
            let actual = paragraph_ranges(text)
                .into_iter()
                .map(|paragraph| (paragraph.content, paragraph.separator))
                .collect::<Vec<_>>();

            assert_eq!(actual, expected, "{text:?}");
        }
    }
}
