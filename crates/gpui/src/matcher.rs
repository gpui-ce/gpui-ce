//! Composable matching of single values and lists.

/// Negates an item or a list expression.
#[derive(Clone, Debug)]
pub struct NotMatch<Input>(Input);

/// Matches any expression in a list.
#[derive(Clone, Debug)]
pub struct AnyMatch<Input>(Input);

/// Negates a value, a collection, or another expression.
pub fn not<Input>(input: Input) -> NotMatch<Input> {
    NotMatch(input)
}

/// Requires at least one expression in an array, vector, slice, or tuple to match.
///
/// This wrapper is only accepted by list matchers.
///
/// ```compile_fail
/// use gpui::{ItemMatch, any};
///
/// let matcher = ItemMatch::<usize>::new(any([1, 2]));
/// ```
///
/// ```compile_fail
/// use gpui::{ListMatch, any};
///
/// let matcher = ListMatch::<usize>::new(any(1));
/// ```
pub fn any<Input>(input: Input) -> AnyMatch<Input> {
    AnyMatch(input)
}

/// Matches one value, optionally negating the comparison.
#[derive(Clone, Debug)]
pub struct ItemMatch<Value> {
    value: Value,
    negated: bool,
}

impl<Value> ItemMatch<Value> {
    /// Creates a matcher from one value or a negated value.
    pub fn new<Kind>(input: impl IntoItemMatch<Value, Kind>) -> Self {
        input.into_item_match()
    }

    /// Returns whether the candidate equals the expected value after negation.
    pub fn matches(&self, candidate: &Value) -> bool
    where
        Value: PartialEq,
    {
        self.evaluate(&|value| candidate == value)
    }

    pub(crate) fn evaluate(&self, predicate: &impl Fn(&Value) -> bool) -> bool {
        predicate(&self.value) != self.negated
    }

    fn negate(mut self) -> Self {
        self.negated = !self.negated;

        self
    }
}

/// Matches values in a list using membership, conjunction, disjunction, and negation.
///
/// Bare arrays, vectors, slices, and tuples require every expression to match.
/// An empty conjunction matches, while an empty disjunction does not.
#[derive(Clone, Debug)]
pub enum ListMatch<Value> {
    /// Tests membership of one value, optionally negated.
    Item(ItemMatch<Value>),
    /// Requires every expression to match.
    All(Vec<Self>),
    /// Requires at least one expression to match.
    Any(Vec<Self>),
    /// Negates the entire expression.
    Not(Box<Self>),
}

impl<Value> ListMatch<Value> {
    /// Creates a matcher from a value, collection, or composed expression.
    pub fn new<Kind>(input: impl IntoListMatch<Value, Kind>) -> Self {
        input.into_list_match()
    }

    /// Returns whether the candidate list satisfies this expression.
    pub fn matches(&self, candidates: &[Value]) -> bool
    where
        Value: PartialEq,
    {
        self.evaluate(&|value| candidates.contains(value))
    }

    fn evaluate(&self, predicate: &impl Fn(&Value) -> bool) -> bool {
        match self {
            Self::Item(item) => item.evaluate(predicate),
            Self::All(expressions) => expressions
                .iter()
                .all(|expression| expression.evaluate(predicate)),
            Self::Any(expressions) => expressions
                .iter()
                .any(|expression| expression.evaluate(predicate)),
            Self::Not(expression) => !expression.evaluate(predicate),
        }
    }
}

/// Converts an input into a matcher for one value.
///
/// `Kind` is inferred and distinguishes value conversion from expression conversion.
pub trait IntoItemMatch<Value, Kind> {
    /// Converts this input into an owned matcher.
    fn into_item_match(self) -> ItemMatch<Value>;
}

/// Converts an input into a matcher for values in a list.
///
/// `Kind` is inferred and distinguishes values, collections, and expressions.
pub trait IntoListMatch<Value, Kind> {
    /// Converts this input into an owned matcher.
    fn into_list_match(self) -> ListMatch<Value>;
}

/// Converts a collection into its individual list expressions.
///
/// `Kind` is inferred from the expressions in the collection.
pub trait IntoMatchList<Value, Kind> {
    /// Converts the collection without choosing conjunction or disjunction.
    fn into_match_list(self) -> Vec<ListMatch<Value>>;
}

/// Converts positive values into an output collection.
///
/// This accepts values and collections without boolean expressions.
/// `Kind` is inferred from the input.
pub trait IntoMatchValues<Value, Kind> {
    /// Appends converted values to the output collection.
    fn extend_match_values(self, output: &mut impl Extend<Value>);
}

/// Distinguishes a converted value from a composed expression.
#[doc(hidden)]
pub struct MatchValue;

/// Distinguishes an already constructed matcher from a converted value.
#[doc(hidden)]
pub struct MatchExpression;

/// Distinguishes a conjunction of collection entries from one value.
#[doc(hidden)]
pub struct MatchCollection<Kind>(std::marker::PhantomData<Kind>);

impl<Value, Input> IntoItemMatch<Value, MatchValue> for Input
where
    Input: Into<Value>,
{
    fn into_item_match(self) -> ItemMatch<Value> {
        ItemMatch {
            value: self.into(),
            negated: false,
        }
    }
}

impl<Value> IntoItemMatch<Value, MatchExpression> for ItemMatch<Value> {
    fn into_item_match(self) -> ItemMatch<Value> {
        self
    }
}

impl<Value, Input, Kind> IntoItemMatch<Value, NotMatch<Kind>> for NotMatch<Input>
where
    Input: IntoItemMatch<Value, Kind>,
{
    fn into_item_match(self) -> ItemMatch<Value> {
        self.0.into_item_match().negate()
    }
}

impl<Value, Input> IntoListMatch<Value, MatchValue> for Input
where
    Input: Into<Value>,
{
    fn into_list_match(self) -> ListMatch<Value> {
        ListMatch::Item(self.into_item_match())
    }
}

impl<Value> IntoListMatch<Value, MatchExpression> for ListMatch<Value> {
    fn into_list_match(self) -> ListMatch<Value> {
        self
    }
}

impl<Value, Input, Kind> IntoListMatch<Value, MatchCollection<Kind>> for Input
where
    Input: IntoMatchList<Value, Kind>,
{
    fn into_list_match(self) -> ListMatch<Value> {
        ListMatch::All(self.into_match_list())
    }
}

impl<Value> IntoListMatch<Value, ItemMatch<Value>> for ItemMatch<Value> {
    fn into_list_match(self) -> ListMatch<Value> {
        ListMatch::Item(self)
    }
}

impl<Value, Input, Kind> IntoListMatch<Value, NotMatch<Kind>> for NotMatch<Input>
where
    Input: IntoListMatch<Value, Kind>,
{
    fn into_list_match(self) -> ListMatch<Value> {
        match self.0.into_list_match() {
            ListMatch::Item(item) => ListMatch::Item(item.negate()),
            ListMatch::Not(expression) => *expression,
            expression => ListMatch::Not(Box::new(expression)),
        }
    }
}

impl<Value, Input, Kind> IntoListMatch<Value, AnyMatch<Kind>> for AnyMatch<Input>
where
    Input: IntoMatchList<Value, Kind>,
{
    fn into_list_match(self) -> ListMatch<Value> {
        ListMatch::Any(self.0.into_match_list())
    }
}

impl<Value, Input> IntoMatchValues<Value, MatchValue> for Input
where
    Input: Into<Value>,
{
    fn extend_match_values(self, output: &mut impl Extend<Value>) {
        output.extend(std::iter::once(self.into()));
    }
}

macro_rules! impl_match_collection {
    ([$input:ident; $count:ident], $kind:ident) => {
        impl<Value, $input, $kind, const $count: usize> IntoMatchList<Value, [$kind; $count]>
            for [$input; $count]
        where
            $input: IntoListMatch<Value, $kind>,
        {
            fn into_match_list(self) -> Vec<ListMatch<Value>> {
                self.into_iter()
                    .map(IntoListMatch::into_list_match)
                    .collect()
            }
        }

        impl<Value, $input, $kind, const $count: usize> IntoMatchValues<Value, [$kind; $count]>
            for [$input; $count]
        where
            $input: IntoMatchValues<Value, $kind>,
        {
            fn extend_match_values(self, output: &mut impl Extend<Value>) {
                for input in self {
                    input.extend_match_values(output);
                }
            }
        }
    };
    ($collection:ident<$input:ident>, $marker:ident<$kind:ident>) => {
        impl<Value, $input, $kind> IntoMatchList<Value, $marker<$kind>> for $collection<$input>
        where
            $input: IntoListMatch<Value, $kind>,
        {
            fn into_match_list(self) -> Vec<ListMatch<Value>> {
                self.into_iter()
                    .map(IntoListMatch::into_list_match)
                    .collect()
            }
        }

        impl<Value, $input, $kind> IntoMatchValues<Value, $marker<$kind>> for $collection<$input>
        where
            $input: IntoMatchValues<Value, $kind>,
        {
            fn extend_match_values(self, output: &mut impl Extend<Value>) {
                for input in self {
                    input.extend_match_values(output);
                }
            }
        }
    };
}

impl_match_collection!([Input; COUNT], Kind);
impl_match_collection!(Vec<Input>, Vec<Kind>);

impl<Value, Input, Kind> IntoMatchList<Value, &[Kind]> for &[Input]
where
    Input: Clone + IntoListMatch<Value, Kind>,
{
    fn into_match_list(self) -> Vec<ListMatch<Value>> {
        self.iter()
            .cloned()
            .map(IntoListMatch::into_list_match)
            .collect()
    }
}

impl<Value, Input, Kind> IntoMatchValues<Value, &[Kind]> for &[Input]
where
    Input: Clone + IntoMatchValues<Value, Kind>,
{
    fn extend_match_values(self, output: &mut impl Extend<Value>) {
        for input in self.iter().cloned() {
            input.extend_match_values(output);
        }
    }
}

impl<Value, Input, Kind, const COUNT: usize> IntoMatchList<Value, &[Kind; COUNT]>
    for &[Input; COUNT]
where
    Input: Clone + IntoListMatch<Value, Kind>,
{
    fn into_match_list(self) -> Vec<ListMatch<Value>> {
        self.as_slice().into_match_list()
    }
}

impl<Value, Input, Kind, const COUNT: usize> IntoMatchValues<Value, &[Kind; COUNT]>
    for &[Input; COUNT]
where
    Input: Clone + IntoMatchValues<Value, Kind>,
{
    fn extend_match_values(self, output: &mut impl Extend<Value>) {
        self.as_slice().extend_match_values(output);
    }
}

macro_rules! impl_match_tuples {
    () => {};
    (
        $head_type:ident => $head_kind:ident => $head:ident
        $(, $tail_type:ident => $tail_kind:ident => $tail:ident)*
        $(,)?
    ) => {
        impl<Value, $head_type, $head_kind $(, $tail_type, $tail_kind)*>
            IntoMatchList<Value, ($head_kind, $($tail_kind,)*)>
            for ($head_type, $($tail_type,)*)
        where
            $head_type: IntoListMatch<Value, $head_kind>,
            $($tail_type: IntoListMatch<Value, $tail_kind>,)*
        {
            fn into_match_list(self) -> Vec<ListMatch<Value>> {
                let ($head, $($tail,)*) = self;

                vec![$head.into_list_match(), $($tail.into_list_match(),)*]
            }
        }

        impl<Value, $head_type, $head_kind $(, $tail_type, $tail_kind)*>
            IntoMatchValues<Value, ($head_kind, $($tail_kind,)*)>
            for ($head_type, $($tail_type,)*)
        where
            $head_type: IntoMatchValues<Value, $head_kind>,
            $($tail_type: IntoMatchValues<Value, $tail_kind>,)*
        {
            fn extend_match_values(self, output: &mut impl Extend<Value>) {
                let ($head, $($tail,)*) = self;
                $head.extend_match_values(output);
                $($tail.extend_match_values(output);)*
            }
        }

        impl_match_tuples!($($tail_type => $tail_kind => $tail),*);
    };
}

impl_match_tuples!(
    First => FirstKind => first,
    Second => SecondKind => second,
    Third => ThirdKind => third,
    Fourth => FourthKind => fourth,
    Fifth => FifthKind => fifth,
    Sixth => SixthKind => sixth,
    Seventh => SeventhKind => seventh,
    Eighth => EighthKind => eighth,
    Ninth => NinthKind => ninth,
    Tenth => TenthKind => tenth,
    Eleventh => EleventhKind => eleventh,
    Twelfth => TwelfthKind => twelfth,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ElementId, SharedString};

    #[test]
    fn composes_list_conditions() {
        let expressions = [
            ListMatch::<SharedString>::new(["apple", "pear"]),
            ListMatch::new(any(["apple", "pear"])),
            ListMatch::new(not(["apple", "pear"])),
            ListMatch::new(not(any(["apple", "pear"]))),
            ListMatch::new(("apple", "pear", not("plum"))),
        ];
        let cases = [
            (vec![], [false, false, true, true, false]),
            (vec!["apple"], [false, true, true, false, false]),
            (vec!["pear"], [false, true, true, false, false]),
            (vec!["apple", "pear"], [true, true, false, false, true]),
            (
                vec!["apple", "pear", "plum"],
                [true, true, false, false, false],
            ),
        ];

        for (classes, expected) in cases {
            let classes = classes
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>();
            let actual = expressions
                .each_ref()
                .map(|matcher| matcher.matches(&classes));

            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn supports_owned_borrowed_and_nested_inputs() {
        let words = ["apple", "pear"];
        let matcher = ListMatch::<SharedString>::new((
            any((String::from("apple"), not("plum"))),
            not(not(words.as_slice())),
        ));
        let classes = words.map(SharedString::from);
        let owned = String::from("apple");
        let borrowed = ListMatch::<SharedString>::new([owned.as_str(), "pear"]);
        drop(owned);

        assert!(matcher.matches(&classes));
        assert!(borrowed.matches(&classes));
        assert!(!matcher.matches(&[]));
        assert!(ListMatch::<i32>::new(vec![1, 3]).matches(&[1, 2, 3]));
        assert!(ListMatch::<i32>::new([] as [i32; 0]).matches(&[]));
        assert!(!ListMatch::<i32>::new(any([] as [i32; 0])).matches(&[]));
        assert!(ListMatch::<i32>::new((1,)).matches(&[1]));
        assert!(
            ListMatch::<i32>::new((1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12))
                .matches(&(1..=12).collect::<Vec<_>>())
        );
    }

    #[test]
    fn item_matching_preserves_value_conversions_and_negation() {
        let named_id = ElementId::from(("row", 3_usize));
        let matcher = ItemMatch::<ElementId>::new(("row", 3_usize));
        let negative = ItemMatch::<ElementId>::new(not(("row", 3_usize)));
        let positive = ItemMatch::<ElementId>::new(not(not(("row", 3_usize))));

        assert!(matcher.matches(&named_id));
        assert!(!negative.matches(&named_id));
        assert!(negative.matches(&ElementId::from("other")));
        assert!(positive.matches(&named_id));
        assert!(ItemMatch::<ElementId>::new([7_u8; 20]).matches(&ElementId::from([7_u8; 20])));
    }
}
