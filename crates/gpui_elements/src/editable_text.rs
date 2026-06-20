mod actions;
mod input_element;
mod input_state;
pub mod notify;
mod shared_state;
mod storage;
mod text_area_element;
mod text_area_state;

pub use actions::*;
pub use input_element::*;
pub use input_state::*;
pub use shared_state::*;
pub use storage::*;
pub use text_area_element::*;
pub use text_area_state::*;

#[allow(dead_code)]
fn make_input(_app: &mut gpui::App) -> impl gpui::IntoElement {
    input(0)
}

#[allow(dead_code)]
fn make_textarea(_app: &mut gpui::App) -> impl gpui::IntoElement {
    text_area(0)
}
