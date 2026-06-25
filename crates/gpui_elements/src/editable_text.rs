pub mod actions;
mod caret;
mod element;
mod history;
mod layout;
mod state;
mod storage;

pub use element::*;
pub use state::*;
pub use storage::*;

/* TODO list
- text sanitation
- add page up/down actions to nav by an entire page or expand selection by an entire page
- test IME (char palette only available on macos)
*/

/* Open questions:
*/
