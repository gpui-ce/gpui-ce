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
- cursor blinking
- text sanitation
- add page up/down actions to nav by an entire page or expand selection by an entire page
- test IME (char palette only available on macos)
*/

/* Open questions:
*/
