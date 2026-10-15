use crate::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, px};
use anyhow::{Ok, Result};

/// A fake display that the test platform reports as its only screen.
///
/// It is 1920 by 1080 logical pixels, with its origin at `(0, 0)`. Each one gets a new UUID.
#[derive(Debug)]
pub(crate) struct TestDisplay {
    id: DisplayId,
    uuid: uuid::Uuid,
    bounds: Bounds<Pixels>,
}

impl TestDisplay {
    /// Creates the fake display, with a new UUID.
    pub fn new() -> Self {
        TestDisplay {
            id: DisplayId(1),
            uuid: uuid::Uuid::new_v4(),
            bounds: Bounds::from_corners(Point::default(), Point::new(px(1920.), px(1080.))),
        }
    }
}

impl PlatformDisplay for TestDisplay {
    fn id(&self) -> crate::DisplayId {
        self.id
    }

    fn uuid(&self) -> Result<uuid::Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> crate::Bounds<crate::Pixels> {
        self.bounds
    }
}
