pub mod actions;
mod element;
mod history;
pub mod notify;
mod state;
mod storage;

pub use element::*;
pub use state::*;
pub use storage::*;

/* TODO list
- remove gpuikit based input
- auto-scroll when cursor moves
- cursor blinking
- color styling configs
- text sanitation
- test IME (char palette only available on macos)
- unit tests
*/

/* Open questions:
*/
