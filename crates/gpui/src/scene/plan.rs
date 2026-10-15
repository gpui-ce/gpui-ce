use super::{AtlasTextureId, BatchIterator, FilterBoundary, Scene};
use smallvec::SmallVec;
use std::ops::Range;

/// Inline capacity of renderer isolation stacks. Deeper groups allocate additional targets.
pub const MAX_FILTER_GROUP_DEPTH: usize = 2;

/// The index of an offscreen target in the renderer's content-filter target pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterTargetIndex(usize);

impl FilterTargetIndex {
    /// Returns the index as a `usize`.
    pub fn as_usize(self) -> usize {
        self.0
    }
}

/// Where the contents of a filter group are rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterRenderTarget {
    /// Render an unmatched group directly into the current target.
    Inline,
    /// Render into a dedicated offscreen target and composite it into the parent.
    Isolated(FilterTargetIndex),
}

impl FilterRenderTarget {
    fn is_isolated(self) -> bool {
        matches!(self, Self::Isolated(_))
    }
}

/// Allocation and draw totals collected while compiling one [`ScenePlan`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScenePlanRequirements {
    /// The number of entries in [`ScenePlan::commands`].
    pub command_count: usize,
    /// The number of instanced draw calls. A path batch that rasterizes counts as two.
    pub instance_batch_count: usize,
    /// The number of path vertices to rasterize into the path target.
    pub path_rasterization_vertex_count: usize,
    /// The number of sprites that composite the rasterized paths.
    pub path_sprite_count: usize,
    /// The number of [`crate::PaintSurface`]s to draw.
    pub surface_count: usize,
    /// The number of [`crate::BackdropFilter`]s to draw.
    pub backdrop_filter_count: usize,
    /// The number of content-filter groups that draw into an isolated target.
    pub isolated_filter_count: usize,
    /// The number of isolated targets needed at the deepest level of nesting.
    pub isolated_target_count: usize,
    /// Whether any path is rasterized into an offscreen target.
    pub uses_path_target: bool,
    /// Whether a backdrop filter or an isolated content-filter group needs an offscreen target.
    pub uses_offscreen_target: bool,
    /// Whether any filter needs a blur target.
    pub uses_blur_target: bool,
}

/// A compiled scene command stream, built by [`Scene::finish`] and shared by every renderer.
#[derive(Debug, Default)]
pub struct ScenePlan {
    // Retain this allocation across `Scene::clear`/`Scene::finish`; scenes are rebuilt every frame.
    pub(super) commands: Vec<RenderCommand>,
    requirements: ScenePlanRequirements,
    scene_lengths: SceneLengths,
}

impl ScenePlan {
    pub(super) fn clear(&mut self) {
        self.commands.clear();
        self.requirements = ScenePlanRequirements::default();
        self.scene_lengths = SceneLengths::default();
    }

    pub(super) fn build(scene: &Scene, mut commands: Vec<RenderCommand>) -> Self {
        commands.clear();
        commands.reserve(scene.len());
        let mut matched_starts =
            SmallVec::<[bool; 8]>::from_elem(false, scene.filter_boundaries.len());
        let mut pending_starts = SmallVec::<[usize; 4]>::new();
        for (index, boundary) in scene.filter_boundaries.iter().enumerate() {
            if boundary.is_start {
                pending_starts.push(index);
            } else if let Some(start_index) = pending_starts.pop() {
                matched_starts[start_index] = true;
            }
        }

        let mut filter_stack = SmallVec::<[(usize, FilterRenderTarget); 4]>::new();
        let mut isolated_depth = 0;
        let mut requirements = ScenePlanRequirements::default();

        for batch in BatchIterator::new(scene) {
            match batch {
                PrimitiveBatch::FilterBoundary(boundary_index) => {
                    let boundary = &scene.filter_boundaries[boundary_index];
                    if boundary.is_start {
                        let target = if matched_starts[boundary_index] {
                            FilterRenderTarget::Isolated(FilterTargetIndex(isolated_depth))
                        } else {
                            FilterRenderTarget::Inline
                        };
                        if target.is_isolated() {
                            isolated_depth += 1;
                            requirements.uses_offscreen_target = true;
                            requirements.uses_blur_target |= boundary.max_blur_radius() > 0.0;
                            requirements.isolated_target_count =
                                requirements.isolated_target_count.max(isolated_depth);
                        }
                        filter_stack.push((boundary_index, target));
                        commands.push(RenderCommand::BeginFilter {
                            boundary_index,
                            target,
                        });
                    } else if let Some((start_index, target)) = filter_stack.pop() {
                        if target.is_isolated() {
                            isolated_depth -= 1;
                            requirements.isolated_filter_count += 1;
                        }
                        commands.push(RenderCommand::EndFilter {
                            boundary_index: start_index,
                            closing_boundary_index: boundary_index,
                            target,
                        });
                    } else {
                        debug_assert!(false, "content-filter end boundary has no matching start");
                        commands.push(RenderCommand::EndFilter {
                            boundary_index,
                            closing_boundary_index: boundary_index,
                            target: FilterRenderTarget::Inline,
                        });
                    }
                }
                batch => {
                    if let PrimitiveBatch::BackdropFilters(range) = &batch {
                        requirements.uses_blur_target |= scene.backdrop_filters[range.clone()]
                            .iter()
                            .any(|filter| filter.max_blur_radius() > 0.0);
                    }

                    requirements.include_batch(&batch);
                    commands.push(RenderCommand::Batch(batch));
                }
            }
        }

        requirements.command_count = commands.len();
        Self {
            commands,
            requirements,
            scene_lengths: SceneLengths::for_scene(scene),
        }
    }

    /// Returns the commands in the order they must be drawn.
    pub fn commands(&self) -> &[RenderCommand] {
        &self.commands
    }

    /// Returns the totals collected while this plan was compiled.
    pub fn requirements(&self) -> &ScenePlanRequirements {
        &self.requirements
    }

    pub(super) fn assert_matches(&self, scene: &Scene) {
        debug_assert_eq!(
            self.scene_lengths,
            SceneLengths::for_scene(scene),
            "scene primitive storage changed after Scene::finish"
        );
    }
}

impl ScenePlanRequirements {
    fn include_batch(&mut self, batch: &PrimitiveBatch) {
        match batch {
            PrimitiveBatch::Shadows { range, .. }
            | PrimitiveBatch::Quads { range, .. }
            | PrimitiveBatch::Underlines(range) => {
                self.instance_batch_count += usize::from(!range.is_empty());
            }
            PrimitiveBatch::Paths {
                rasterization_vertex_count,
                sprite_count,
                ..
            } => {
                self.path_rasterization_vertex_count += rasterization_vertex_count;
                if *rasterization_vertex_count > 0 {
                    self.path_sprite_count += sprite_count;
                    self.uses_path_target = true;
                    self.instance_batch_count += 2;
                }
            }
            PrimitiveBatch::MonochromeSprites { range, .. }
            | PrimitiveBatch::SubpixelSprites { range, .. }
            | PrimitiveBatch::PolychromeSprites { range, .. } => {
                self.instance_batch_count += usize::from(!range.is_empty());
            }
            PrimitiveBatch::Surfaces(range) => self.surface_count += range.len(),
            PrimitiveBatch::BackdropFilters(range) => {
                self.backdrop_filter_count += range.len();
                self.uses_offscreen_target |= !range.is_empty();
            }
            PrimitiveBatch::FilterBoundary(_) => {
                unreachable!("filter boundaries are compiled before requirements are collected")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SceneLengths {
    shadows: usize,
    quads: usize,
    paths: usize,
    underlines: usize,
    monochrome_sprites: usize,
    subpixel_sprites: usize,
    polychrome_sprites: usize,
    surfaces: usize,
    backdrop_filters: usize,
    filter_boundaries: usize,
}

impl SceneLengths {
    fn for_scene(scene: &Scene) -> Self {
        Self {
            shadows: scene.shadows.len(),
            quads: scene.quads.len(),
            paths: scene.paths.len(),
            underlines: scene.underlines.len(),
            monochrome_sprites: scene.monochrome_sprites.len(),
            subpixel_sprites: scene.subpixel_sprites.len(),
            polychrome_sprites: scene.polychrome_sprites.len(),
            surfaces: scene.surfaces.len(),
            backdrop_filters: scene.backdrop_filters.len(),
            filter_boundaries: scene.filter_boundaries.len(),
        }
    }
}

/// A contiguous run of one primitive type that the renderer draws in one call.
///
/// The ranges index the matching vector in [`Scene`]. A batch never mixes primitive types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrimitiveBatch {
    /// Shadows, drawn with or without corner smoothing as a unit.
    Shadows {
        /// The indices into [`Scene::shadows`].
        range: Range<usize>,
        /// Whether the shadows use corner smoothing.
        smoothed: bool,
    },
    /// Quads, drawn with or without corner smoothing as a unit.
    Quads {
        /// The indices into [`Scene::quads`].
        range: Range<usize>,
        /// Whether the quads use corner smoothing.
        smoothed: bool,
    },
    /// Paths. The renderer rasterizes their vertices, then composites the results as sprites.
    Paths {
        /// The indices into [`Scene::paths`].
        range: Range<usize>,
        /// The number of path vertices to rasterize in this batch.
        rasterization_vertex_count: usize,
        /// The number of sprites that composite the rasterized paths.
        sprite_count: usize,
    },
    /// Underlines, as indices into [`Scene::underlines`].
    Underlines(Range<usize>),
    /// Single-color sprites that all read from one atlas texture.
    MonochromeSprites {
        /// The atlas texture that the sprites read from.
        texture_id: AtlasTextureId,
        /// The indices into [`Scene::monochrome_sprites`].
        range: Range<usize>,
    },
    /// Subpixel-antialiased sprites that all read from one atlas texture.
    SubpixelSprites {
        /// The atlas texture that the sprites read from.
        texture_id: AtlasTextureId,
        /// The indices into [`Scene::subpixel_sprites`].
        range: Range<usize>,
    },
    /// Full-color sprites that all read from one atlas texture.
    PolychromeSprites {
        /// The atlas texture that the sprites read from.
        texture_id: AtlasTextureId,
        /// The indices into [`Scene::polychrome_sprites`].
        range: Range<usize>,
        /// Whether the sprites use corner smoothing.
        smoothed: bool,
    },
    /// Surfaces, as indices into [`Scene::surfaces`].
    Surfaces(Range<usize>),
    /// Backdrop filters, as indices into [`Scene::backdrop_filters`].
    BackdropFilters(Range<usize>),
    /// A content-filter boundary, as an index into [`Scene::filter_boundaries`].
    FilterBoundary(usize),
}

/// Backend-neutral rendering work derived from a [`Scene`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderCommand {
    /// A normal primitive batch.
    Batch(PrimitiveBatch),
    /// Begin rendering a content-filter group.
    BeginFilter {
        /// Index of the opening boundary in [`Scene::filter_boundaries`].
        boundary_index: usize,
        /// Whether this group renders inline or into an isolated target.
        target: FilterRenderTarget,
    },
    /// Finish and composite a content-filter group.
    EndFilter {
        /// Index of the matched opening boundary in [`Scene::filter_boundaries`].
        boundary_index: usize,
        /// Index of the closing marker in [`Scene::filter_boundaries`].
        closing_boundary_index: usize,
        /// The same target selected by the matching begin command.
        target: FilterRenderTarget,
    },
}

impl RenderCommand {
    /// Returns the opening [`FilterBoundary`] of a filter command, or `None` for a batch.
    pub fn boundary<'a>(&self, scene: &'a Scene) -> Option<&'a FilterBoundary> {
        match self {
            Self::BeginFilter { boundary_index, .. } | Self::EndFilter { boundary_index, .. } => {
                Some(&scene.filter_boundaries[*boundary_index])
            }
            Self::Batch(_) => None,
        }
    }

    /// Returns a short description of this command, for GPU debugger labels.
    pub fn label(&self) -> String {
        match self {
            Self::Batch(batch) => batch.label(),
            Self::BeginFilter { target, .. } => match target {
                FilterRenderTarget::Isolated(index) => {
                    format!("begin filter group ({})", index.as_usize())
                }
                FilterRenderTarget::Inline => "begin inline filter group".into(),
            },
            Self::EndFilter { target, .. } => match target {
                FilterRenderTarget::Isolated(index) => {
                    format!("end filter group ({})", index.as_usize())
                }
                FilterRenderTarget::Inline => "end inline filter group".into(),
            },
        }
    }
}

impl PrimitiveBatch {
    /// Returns a short description of this batch, for GPU debugger labels.
    pub fn label(&self) -> String {
        match self {
            Self::Shadows { range, smoothed } => format!(
                "{}shadows ({})",
                if *smoothed { "smoothed " } else { "" },
                range.len()
            ),
            Self::Quads { range, smoothed } => format!(
                "{}quads ({})",
                if *smoothed { "smoothed " } else { "" },
                range.len()
            ),
            Self::Paths { range, .. } => format!("paths ({})", range.len()),
            Self::Underlines(range) => format!("underlines ({})", range.len()),
            Self::MonochromeSprites { texture_id, range } => format!(
                "monochrome sprites ({}) on atlas {}",
                range.len(),
                texture_id.index
            ),
            Self::SubpixelSprites { texture_id, range } => format!(
                "subpixel sprites ({}) on atlas {}",
                range.len(),
                texture_id.index
            ),
            Self::PolychromeSprites {
                texture_id,
                range,
                smoothed,
            } => format!(
                "{}polychrome sprites ({}) on atlas {}",
                if *smoothed { "smoothed " } else { "" },
                range.len(),
                texture_id.index
            ),
            Self::Surfaces(range) => format!("surfaces ({})", range.len()),
            Self::BackdropFilters(range) => format!("backdrop filters ({})", range.len()),
            Self::FilterBoundary(index) => format!("filter boundary ({index})"),
        }
    }
}
