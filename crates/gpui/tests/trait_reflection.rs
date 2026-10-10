#[path = "../examples/learn/trait_reflection.rs"]
#[allow(dead_code)]
mod component_example;

#[cfg(test)]
#[macro_use]
#[path = "../src/reflection/element_test_support.rs"]
mod element_test_support;

#[cfg(test)]
mod tests {
    use crate::component_example::{
        CardElement, Draggable as ComponentDraggable, Icon, set_drag_payload,
    };
    #[cfg(any(reflection_schema, reflection_parent_schema))]
    pub use gpui::__GpuiReflectStyledSchema as __GpuiReflectWrongAliasSchema;
    #[cfg(any(reflection_schema, reflection_parent_schema))]
    use gpui::ParentElement as WrongAlias;
    use gpui::reflection::{
        CallableReflectionToken, IncludesCallableTrait, IncludesReflectedTrait, Reflect,
        ReflectedElement, ReflectedTraits, ReflectionToken,
    };
    use gpui::{
        Animation, AnimationElement, AnimationExt, AnyElement, Div, Element, InteractiveElement,
        Interactivity, IntoElement, ParentElement, SharedString, SpringAnimation,
        SpringAnimationElement, SpringConfig, StatefulInteractiveElement, StyleRefinement, Styled,
        ViewElement, div, px,
    };
    use std::{any::TypeId, time::Duration};

    #[gpui::reflection::reflect_trait]
    trait Draggable {
        fn distance(&mut self) -> &mut usize;

        fn label(&self) -> &str {
            "default"
        }

        fn drag(&mut self, distance: usize) -> usize {
            let total = self.distance();
            *total += distance;

            *total
        }

        #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
        fn unavailable(&self) -> UnavailableType;

        fn draggable(self) -> Self
        where
            Self: Sized,
        {
            self
        }
    }

    mod other {
        #[gpui::reflection::reflect_trait]
        pub trait Draggable {}

        #[gpui::reflection::reflect_trait]
        pub trait Styled: Sized {}

        #[gpui::reflection::reflect_trait(membership)]
        #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
        trait Unavailable: MissingParent {
            fn missing<Input>(&self, input: Input) -> MissingType;
        }
    }

    #[allow(private_bounds)]
    mod inherited {
        #[gpui::reflection::reflect_trait]
        trait PrivateParent {}

        #[gpui::reflection::reflect_trait]
        pub(crate) trait CrateParent: self::PrivateParent {}

        #[gpui::reflection::reflect_trait]
        pub trait Control: gpui::StatefulInteractiveElement {}

        pub use self::__GpuiReflectControlSchema as __GpuiReflectPublicControlSchema;
        pub use self::Control as PublicControl;
        pub use self::Control as SingleControl;

        #[gpui::reflection::reflect_trait]
        pub trait AliasControl: self::PublicControl {}

        #[gpui::reflection::reflect_trait]
        pub trait SealedControl: self::CrateParent {}
    }

    #[cfg(reflection_parent_schema)]
    #[gpui::reflection::reflect_trait]
    trait WrongParent: WrongAlias {}

    #[derive(gpui::reflection::Reflect, Styled, ParentElement, Default)]
    struct Card {
        #[style]
        style: StyleRefinement,
        #[children]
        children: Vec<AnyElement>,
    }

    #[derive(gpui::reflection::Reflect, Default)]
    #[reflect(Draggable, inherited::Control)]
    struct Control {
        interactivity: Interactivity,
        distance: usize,
    }

    impl gpui::InteractiveElement for Control {
        fn interactivity(&mut self) -> &mut Interactivity {
            &mut self.interactivity
        }
    }

    impl gpui::StatefulInteractiveElement for Control {}

    impl Draggable for Control {
        fn distance(&mut self) -> &mut usize {
            &mut self.distance
        }

        fn label(&self) -> &str {
            "control"
        }

        fn drag(&mut self, distance: usize) -> usize {
            self.distance += distance * 2;

            self.distance
        }
    }

    #[derive(gpui::reflection::Reflect, Default)]
    #[reflect(Draggable)]
    struct DefaultControl {
        distance: usize,
    }

    impl Draggable for DefaultControl {
        fn distance(&mut self) -> &mut usize {
            &mut self.distance
        }
    }

    impl inherited::Control for Control {}

    impl other::Draggable for Control {}

    element_impl!(Card);

    element_impl!(Control);

    #[gpui::reflection::reflect_trait(membership)]
    trait PaintSource: std::fmt::Debug + Send {
        type Brush;
        const PALETTE_SIZE: usize;

        fn paint<Input>(&self, input: Input) -> Self::Brush;
    }

    #[cfg(reflection_parent)]
    #[gpui::reflection::reflect_trait]
    trait CallablePaintSource: PaintSource {}

    #[derive(gpui::reflection::Reflect, Debug)]
    #[expect(
        clippy::duplicated_attributes,
        reason = "Duplicate traits exercise reflection registration deduplication."
    )]
    #[reflect(gpui::Styled, PaintSource, PaintSource)]
    struct GenericCard<State, const COUNT: usize>
    where
        State: std::fmt::Debug,
    {
        state: State,
        style: StyleRefinement,
    }

    impl<State: std::fmt::Debug + Send, const COUNT: usize> PaintSource for GenericCard<State, COUNT> {
        type Brush = usize;
        const PALETTE_SIZE: usize = COUNT;

        fn paint<Input>(&self, _input: Input) -> usize {
            Self::PALETTE_SIZE
        }
    }

    impl<State: std::fmt::Debug, const COUNT: usize> gpui::Styled for GenericCard<State, COUNT> {
        fn style(&mut self) -> &mut StyleRefinement {
            &mut self.style
        }
    }

    element_impl!(GenericCard<State, COUNT>, [State: std::fmt::Debug + Send + 'static, const COUNT: usize],
        reflection <Self as gpui::reflection::Reflect>::reflection());

    fn methods<Type: Draggable + 'static>() -> Box<__GpuiReflectDraggableMethods> {
        let mut implementations = Vec::new();
        Draggable.__register::<Type>(&mut implementations);

        implementations
            .pop()
            .unwrap()
            .methods
            .unwrap()
            .downcast()
            .unwrap()
    }

    #[test]
    fn exposes_generic_and_membership_registrations_through_erasure() {
        fn require_mixed_group<Traits>(
            traits: Traits,
        ) -> Vec<gpui::reflection::ReflectionRequirement>
        where
            Traits: ReflectedTraits,
            Traits::Group: IncludesReflectedTrait<__GpuiReflectPaintSource>
                + IncludesCallableTrait<gpui::__GpuiReflectStyled>,
            ReflectedElement<Traits::Group>: gpui::Styled,
        {
            traits.reflected_requirements().into_iter().collect()
        }

        let mut card = GenericCard::<String, 4> {
            state: "state".into(),
            style: StyleRefinement::default(),
        }
        .into_any_element();
        let metadata = card.reflection();

        assert!(card.implements_trait(PaintSource));
        assert!(card.implements_trait(gpui::Styled));
        assert!(metadata.has_adapter(gpui::Styled));
        assert!(!PaintSource.requires_adapter());
        assert!((PaintSource.reflected_trait().supertraits)().is_empty());
        assert_eq!(metadata.descriptors().len(), 2);

        let requirements = require_mixed_group(gpui::reflection::trait_set![
            PaintSource,
            gpui::Styled,
            PaintSource,
            gpui::Styled,
        ]);

        assert_eq!(requirements.len(), 2);
        assert_eq!(requirements[0], PaintSource.requirement());
        assert_eq!(requirements[1], gpui::Styled.requirement());
        assert!(
            requirements
                .iter()
                .all(|requirement| metadata.satisfies(*requirement))
        );
        assert!(std::ptr::eq(
            metadata,
            <GenericCard<String, 4> as Reflect>::reflection()
        ));
        assert_eq!(
            gpui::reflection::registered_traits(std::any::TypeId::of::<GenericCard<String, 4>>()),
            metadata.descriptors()
        );

        let concrete = card.downcast_mut::<GenericCard<String, 4>>().unwrap();

        assert_eq!(concrete.state, "state");
        assert_eq!(PaintSource::paint(concrete, "input"), 4);
        assert_eq!(GenericCard::<String, 4>::PALETTE_SIZE, 4);

        let other = GenericCard::<u32, 8> {
            state: 42,
            style: StyleRefinement::default(),
        }
        .into_any_element();

        assert!(!std::ptr::eq(metadata, other.reflection()));
        assert_eq!(
            other.reflection().concrete_type(),
            Some(std::any::TypeId::of::<GenericCard<u32, 8>>())
        );
    }

    #[test]
    fn reflects_traits() {
        for (requirements, requirement) in [
            (Draggable.requirements(), Draggable.requirement()),
            (
                other::Draggable.requirements(),
                other::Draggable.requirement(),
            ),
            (other::Styled.requirements(), other::Styled.requirement()),
        ] {
            assert_eq!(requirements, vec![requirement]);
            assert!(requirement.requires_adapter);
            assert!((requirement.descriptor.supertraits)().is_empty());
        }

        fn require_other_draggable<Type: other::Draggable>() {}

        fn require_callable_group<Group, Token>(_group: Group, _token: Token)
        where
            Token: CallableReflectionToken,
            Group: IncludesCallableTrait<Token>,
        {
        }

        fn require_selected_traits<Traits>(traits: Traits)
        where
            Traits: gpui::reflection::ReflectedTraits,
            gpui::reflection::ReflectedElement<Traits::Group>:
                gpui::Element + gpui::Styled + gpui::ParentElement + Draggable + other::Styled,
        {
            drop(traits);
        }

        fn require_inherited_traits<Traits>(traits: Traits)
        where
            Traits: gpui::reflection::ReflectedTraits,
            gpui::reflection::ReflectedElement<Traits::Group>:
                inherited::Control + gpui::StatefulInteractiveElement + gpui::InteractiveElement,
        {
            drop(traits);
        }

        fn require_alias_and_sealed_traits<Traits>(traits: Traits)
        where
            Traits: ReflectedTraits,
            ReflectedElement<Traits::Group>: inherited::AliasControl + inherited::SealedControl,
        {
            assert_eq!(traits.reflected_requirements().len(), 7);
        }

        require_other_draggable::<Control>();
        require_callable_group(__GpuiReflectDraggableGroup, Draggable);
        require_callable_group(
            inherited::__GpuiReflectControlGroup,
            gpui::InteractiveElement,
        );
        require_selected_traits(gpui::reflection::trait_set!(
            gpui::Styled,
            gpui::ParentElement,
            crate::tests::Draggable,
            other::Styled,
        ));
        require_inherited_traits(inherited::Control);
        require_inherited_traits(inherited::SingleControl);
        require_inherited_traits(gpui::reflection::trait_set!(
            inherited::PublicControl,
            gpui::InteractiveElement,
            gpui::StatefulInteractiveElement,
            inherited::Control,
        ));
        require_alias_and_sealed_traits(gpui::reflection::trait_set![
            inherited::AliasControl,
            inherited::SealedControl,
        ]);

        assert_eq!(
            inherited::SingleControl.requirements(),
            inherited::Control.requirements(),
        );

        let builtin = gpui::reflection::trait_set![gpui::Styled, gpui::InteractiveElement];

        assert_eq!(builtin.reflected_requirements().len(), 2);
        assert_ne!(
            gpui::Styled.reflected_trait(),
            other::Styled.reflected_trait()
        );

        let card = Card::default().into_any_element();

        assert!(card.implements_trait(gpui::Styled));
        assert!(card.implements_trait(gpui::ParentElement));
        assert!(!card.implements_trait(gpui::InteractiveElement));
        assert!(!card.implements_trait(Draggable));

        let control = Control::default().draggable().into_any_element();

        assert!(control.implements_trait(gpui::InteractiveElement));
        assert!(control.implements_trait(gpui::StatefulInteractiveElement));
        assert!(control.implements_trait(Draggable));
        assert!(control.implements_trait(inherited::Control));
        assert!(!control.implements_trait(other::Draggable));
        assert!(!control.implements_trait(gpui::Styled));
    }

    #[test]
    fn dispatches_registered_defaults_and_overrides() {
        let mut control = Control::default();
        let mut inherited = DefaultControl::default();

        let concrete_methods = methods::<Control>();
        let inherited_methods = methods::<DefaultControl>();

        assert_eq!((concrete_methods.label)(&control), "control");
        assert_eq!((inherited_methods.label)(&inherited), "default");
        assert_eq!((concrete_methods.drag)(&mut control, 3), 6);
        assert_eq!((inherited_methods.drag)(&mut inherited, 3), 3);

        assert_eq!((control.distance, inherited.distance), (6, 3));
    }

    #[test]
    fn component_and_wrapper_reflection_use_their_concrete_receivers() {
        fn unreflected<Type: Element>(element: Type) -> AnyElement {
            let mut element = element.into_any();

            assert!(element.downcast_mut::<Type>().is_some());

            element
        }

        let card = CardElement::new("card")
            .size(px(80.))
            .role(accesskit::Role::Button)
            .aria_label("Card")
            .aria_hidden()
            .child(div());

        assert_eq!(Element::id(&card), Some("card".into()));
        assert_eq!(card.a11y_role(), Some(accesskit::Role::Button));
        assert!(card.is_a11y_hidden());
        assert_eq!(card.source_location().is_some(), cfg!(debug_assertions));

        let mut node = accesskit::Node::new(accesskit::Role::Button);
        card.write_a11y_info(&mut node);
        assert_eq!(node.label(), Some("Card"));

        let mut element = card.into_any_element();

        assert_eq!(
            element.reflection().concrete_type(),
            Some(TypeId::of::<CardElement>())
        );

        for requirement in [
            gpui::Styled.requirement(),
            gpui::ParentElement.requirement(),
            gpui::InteractiveElement.requirement(),
            gpui::StatefulInteractiveElement.requirement(),
            ComponentDraggable.requirement(),
        ] {
            assert!(element.reflection().satisfies(requirement));
        }

        set_drag_payload(&mut element, "public-path");
        assert_eq!(
            element.downcast_mut::<CardElement>().unwrap().drag_payload,
            Some("public-path".into())
        );

        let mut stateful_div = div().id("card").into_any_element();

        assert!(stateful_div.implements_trait(gpui::InteractiveElement));
        assert!(!stateful_div.implements_trait(gpui::StatefulInteractiveElement));
        assert_eq!(
            Element::id(stateful_div.downcast_mut::<Div>().unwrap()),
            Some("card".into())
        );

        let mut icon = Icon::default().into_any_element();

        assert!(<Icon as Reflect>::reflection().implements_trait(ComponentDraggable));
        assert!(icon.downcast_mut::<ViewElement<Icon>>().is_some());

        let mut animation = CardElement::new("card")
            .with_animation(
                "animation",
                Animation::new(Duration::from_secs(1)),
                |element, _progress| element,
            )
            .into_any_element();
        let mut spring = CardElement::new("card")
            .with_spring(
                "spring",
                SpringAnimation::new(SpringConfig::new(100., 10., 1.)).to(1.),
                |element, _progress| element,
            )
            .into_any_element();

        assert!(
            animation
                .downcast_mut::<AnimationElement<CardElement>>()
                .is_some()
        );
        assert!(
            spring
                .downcast_mut::<SpringAnimationElement<CardElement>>()
                .is_some()
        );

        for element in [
            icon,
            animation,
            spring,
            unreflected("text"),
            unreflected(SharedString::from("text")),
        ] {
            assert_eq!(element.reflection().concrete_type(), None);
            assert!(element.reflected_traits().is_empty());
            assert!(!element.implements_trait(ComponentDraggable));
            assert!(!element.implements_trait(gpui::Styled));
            assert!(!element.implements_trait(gpui::StatefulInteractiveElement));
        }
    }

    #[cfg(any(
        reflection_bounds,
        reflection_token,
        reflection_method,
        reflection_type
    ))]
    #[test]
    fn rejects_unavailable_membership_capabilities() {
        #[cfg(reflection_bounds)]
        let _metadata = <GenericCard<std::rc::Rc<usize>, 4> as Reflect>::reflection();

        #[cfg(reflection_token)]
        {
            fn require_callable<Token: gpui::reflection::CallableReflectionToken>(_token: Token) {}

            require_callable(PaintSource);
        }

        #[cfg(reflection_method)]
        fn paint<Group: IncludesReflectedTrait<__GpuiReflectPaintSource>>(
            element: &ReflectedElement<Group>,
        ) {
            PaintSource::paint(element, "input");
        }

        #[cfg(reflection_type)]
        let _brush: Option<
            <gpui::reflection::ReflectedElement<gpui::reflection::ErasedReflectionGroup> as PaintSource>::Brush,
        > = None;
    }

    #[cfg(any(reflection_schema, reflection_identity, reflection_private))]
    #[test]
    fn rejects_invalid_group_proofs() {
        #[cfg(reflection_schema)]
        {
            let _single = gpui::reflection::trait_set![WrongAlias];
            let _repeated = gpui::reflection::trait_set![gpui::Styled, WrongAlias];
        }

        #[cfg(reflection_identity)]
        {
            fn require_other_styled<Traits>(traits: Traits)
            where
                Traits: ReflectedTraits,
                Traits::Group: IncludesCallableTrait<other::__GpuiReflectStyled>,
            {
                drop(traits);
            }

            require_other_styled(gpui::reflection::trait_set![gpui::Styled]);
        }

        #[cfg(reflection_private)]
        let _parent = inherited::PrivateParent;
    }
}
