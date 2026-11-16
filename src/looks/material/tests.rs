use super::*;

#[test]
fn frontier_supports_colour_stocks_and_none_but_not_monochrome_stocks() {
    for variant in frontier::Variant::all() {
        let paper = Paper::Frontier(variant);
        let none = Selection::new(None, paper).unwrap();
        assert_eq!(none.negative(), None);
        assert_eq!(none.paper(), paper);
        for stock in spektrafilm::Negative::all() {
            let negative = Negative::Colour(stock);
            let selection = Selection::new(Some(negative), paper).unwrap();
            assert!(paper.supports_negative(negative));
            assert_eq!(selection.negative(), Some(negative));
            assert_eq!(selection.paper(), paper);
            assert_eq!(Selection::from_origin(&selection.origin()), Some(selection));
        }
        for stock in monochrome::Negative::all() {
            assert!(!paper.supports_negative(Negative::Monochrome(stock)));
            assert!(Selection::new(Some(Negative::Monochrome(stock)), paper).is_none());
        }
    }
}
