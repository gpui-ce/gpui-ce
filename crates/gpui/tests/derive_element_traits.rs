use gpui::{
    AnyElement, Empty, Interactivity, IntoElement, ParentElement, StatefulInteractiveElement,
    StyleRefinement, Styled,
};

#[derive(gpui::Styled, gpui::ParentElement)]
struct Card {
    #[style]
    style: StyleRefinement,
    #[children]
    children: Vec<AnyElement>,
}

#[derive(gpui::InteractiveElement, gpui::StatefulInteractiveElement)]
struct Control {
    #[interactivity]
    interactivity: Interactivity,
}

#[derive(
    gpui::Styled, gpui::ParentElement, gpui::InteractiveElement, gpui::StatefulInteractiveElement,
)]
struct Delegated<StyleTarget, InteractiveTarget>(
    #[style(delegate)]
    #[children(delegate)]
    StyleTarget,
    #[interactivity(delegate)] InteractiveTarget,
)
where
    StyleTarget: Styled + ParentElement,
    InteractiveTarget: gpui::InteractiveElement;

fn requires_stateful<Type: StatefulInteractiveElement>(element: &mut Type) -> &mut Interactivity {
    element.interactivity()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_element_traits() {
        let mut card = Card {
            style: StyleRefinement::default(),
            children: Vec::new(),
        };

        let style: &mut StyleRefinement = card.style();
        style.opacity = Some(0.5);
        card.extend([Empty.into_any_element()]);
        assert_eq!(card.children.len(), 1);

        let mut control = Control {
            interactivity: Interactivity::default(),
        };
        let expected = std::ptr::from_ref(&control.interactivity);

        assert!(std::ptr::eq(requires_stateful(&mut control), expected));

        let mut delegated = Delegated(card, control);
        delegated.style().opacity = Some(0.75);
        delegated.extend([Empty.into_any_element()]);

        assert_eq!(delegated.0.style.opacity, Some(0.75));
        assert_eq!(delegated.0.children.len(), 2);

        let original = std::ptr::from_ref(&delegated.1.interactivity);
        let interactivity: &mut Interactivity = requires_stateful(&mut delegated);

        assert!(std::ptr::eq(interactivity, original));
    }
}
