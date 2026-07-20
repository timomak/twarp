use super::{ease_out_cubic, paint_shift, SlideEdge};

#[test]
fn cubic_ease_clamps_and_reaches_expected_midpoint() {
    assert_eq!(ease_out_cubic(-1.0), 0.0);
    assert_eq!(ease_out_cubic(0.0), 0.0);
    assert_eq!(ease_out_cubic(0.5), 0.875);
    assert_eq!(ease_out_cubic(1.0), 1.0);
    assert_eq!(ease_out_cubic(2.0), 1.0);
}

#[test]
fn paint_shift_moves_each_rail_toward_its_window_edge() {
    assert_eq!(paint_shift(SlideEdge::Left, 25.0, 100.0), -75.0);
    assert_eq!(paint_shift(SlideEdge::Right, 25.0, 100.0), 0.0);
}
