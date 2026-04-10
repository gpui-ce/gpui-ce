#[test]
fn test_derive_render() {
    use gpui_ce_macros::Render;

    #[derive(Render)]
    struct _Element;
}
