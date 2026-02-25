use crate::PlatformKeyboardLayout;

pub(crate) struct CrossKeyboardLayout;

impl PlatformKeyboardLayout for CrossKeyboardLayout {
    fn id(&self) -> &str {
        // TODO(mdeand): I'm not quite sure what logic needs to happen for the cross platform, so for now - us.
        "us"
    }

    fn name(&self) -> &str {
        // TODO(mdeand): I'm not quite sure what logic needs to happen for the cross platform, so for now - us.
        "us"
    }
}