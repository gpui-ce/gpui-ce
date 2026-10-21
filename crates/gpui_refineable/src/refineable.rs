pub use derive_refineable::Refineable;

/// Types that support partial updates, such as styles, settings, and themes.
///
/// Refinements selectively override a base configuration to build configuration hierarchies.
///
/// `#[derive(Refineable)]` implements this trait and creates a companion type, such as
/// `StyleRefinement` for `Style`:
///
/// - Fields marked `#[refineable]` use their refinement type. `Bar` becomes `BarRefinement`,
///   and `BarRefinement` remains unchanged.
/// - `Option<T>` fields keep their type.
/// - Other fields become `Option<T>`.
///
/// Struct attributes control the refinement type:
///
/// - `#[refineable(Debug)]` implements `Debug`.
/// - `#[refineable(Serialize)]` derives `Serialize` and skips `None` values.
/// - `#[refineable(OtherTrait)]` derives additional traits.
pub trait Refineable: Clone {
    type Refinement: Refineable<Refinement = Self::Refinement> + IsEmpty + Default;

    /// Applies non-empty refinement values in place.
    ///
    /// Calls `refine` recursively for refineable fields and replaces other fields
    /// when a value is present.
    fn refine(&mut self, refinement: &Self::Refinement);

    /// Returns this instance with the refinement applied, equivalent to calling `refine`
    /// on an owned value. Clone the instance first to keep the original.
    fn refined(self, refinement: Self::Refinement) -> Self;

    /// Creates an instance from a cascade by merging all refinements atop the default value.
    fn from_cascade(cascade: &Cascade<Self>) -> Self
    where
        Self: Default + Sized,
    {
        Self::default().refined(cascade.merged())
    }

    /// Returns whether this instance contains all refinement values.
    ///
    /// Checks `is_superset_of` recursively for refineable fields and compares `Some`
    /// values for other fields.
    fn is_superset_of(&self, refinement: &Self::Refinement) -> bool;

    /// Returns the difference from the refinement.
    ///
    /// Calls `subtract` recursively for refineable fields and sets other fields to
    /// `None` when their values match the refinement.
    fn subtract(&self, refinement: &Self::Refinement) -> Self::Refinement;
}

pub trait IsEmpty {
    /// Returns `true` if applying this refinement would have no effect.
    fn is_empty(&self) -> bool;
}

/// Optional refinements merged in order, with later slots taking precedence.
/// Slot 0 always contains the base refinement.
///
/// This supports configuration hierarchies such as CSS styles from user-agent,
/// user, and author sources.
pub struct Cascade<S: Refineable>(Vec<Option<S::Refinement>>);

impl<S: Refineable + Default> Default for Cascade<S> {
    fn default() -> Self {
        Self(vec![Some(Default::default())])
    }
}

/// Identifies a cascade slot whose refinement can be set or updated.
#[derive(Copy, Clone)]
pub struct CascadeSlot(usize);

impl<S: Refineable + Default> Cascade<S> {
    /// Reserves an empty `None` slot. Use `set()` to populate it.
    pub fn reserve(&mut self) -> CascadeSlot {
        self.0.push(None);
        CascadeSlot(self.0.len() - 1)
    }

    /// Returns a mutable reference to the base refinement in slot 0, which is always present.
    pub fn base(&mut self) -> &mut S::Refinement {
        self.0[0].as_mut().unwrap()
    }

    /// Sets a slot's refinement. `None` excludes the slot from merging.
    pub fn set(&mut self, slot: CascadeSlot, refinement: Option<S::Refinement>) {
        self.0[slot.0] = refinement
    }

    /// Merges refinements in order, skipping `None` slots. Later slots take precedence.
    pub fn merged(&self) -> S::Refinement {
        let mut merged = self.0[0].clone().unwrap();
        for refinement in self.0.iter().skip(1).flatten() {
            merged.refine(refinement);
        }
        merged
    }
}
