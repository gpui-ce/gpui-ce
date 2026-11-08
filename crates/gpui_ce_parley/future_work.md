# Future work

Parley 0.12.0 integration is split across the migration stack.

## Vertical alignment

GPUI delegates `baseline`, `middle`, `top`, `bottom`, and line metrics to Parley.
The inline container supplies its font for alignment and the line strut. The
manual `align_inline_boxes` pass and caller-provided inline metrics are removed.
Child-provided baselines are part of the atomic sizing layer.

## Whitespace at wrapped line starts

Parley 0.12.0 provides CSS whitespace processing through `TreeBuilder`. The CSS
formatting layer connects this to nested GPUI spans while preserving source byte
indices. This first layer retains the existing flat input and whitespace behavior.

## Out-of-flow boxes

[`InlineBoxKind::OutOfFlow`](https://docs.rs/parley/0.12.0/parley/enum.InlineBoxKind.html#variant.OutOfFlow)
provides static positions without affecting text flow. The absolute positioning
layer connects these anchors to Taffy's sizing, insets, and final placement.
