pub mod actions;
mod input_element;
mod input_state;
pub mod notify;
mod shared_element;
mod shared_state;
mod storage;
mod text_area_element;
mod text_area_state;

pub use input_element::*;
pub use input_state::*;
pub use shared_state::*;
pub use storage::*;
pub use text_area_element::*;
pub use text_area_state::*;

/* TODO list
- default-value
- naming clean up (EditableText)
- remove gpuikit based input
- cursor blinking
- color styling configs
- undo/redo
- text sanitation
- test IME (char palette only available on macos)
- unit tests
*/
