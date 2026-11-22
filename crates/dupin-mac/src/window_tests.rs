use super::*;

#[test]
fn mouse_selection_uses_the_pointer_and_falls_back_to_primary() {
    let screens = [[0.0, 0.0, 1000.0, 800.0], [1000.0, 0.0, 1200.0, 900.0]];
    assert_eq!(
        select_screen(
            &screens,
            (1100.0, 300.0),
            None,
            dupin_core::DisplaySelection::Mouse
        ),
        Some((1, false))
    );
    assert_eq!(
        select_screen(
            &screens,
            (1100.0, 900.0),
            None,
            dupin_core::DisplaySelection::Mouse
        ),
        Some((1, false))
    );
    assert_eq!(
        select_screen(
            &screens,
            (-50.0, 300.0),
            None,
            dupin_core::DisplaySelection::Mouse
        ),
        Some((0, false))
    );
}

#[test]
fn main_selection_uses_the_first_screen() {
    let screens = [[0.0, 0.0, 1000.0, 800.0], [-1200.0, 100.0, 1200.0, 800.0]];
    assert_eq!(
        select_screen(
            &screens,
            (-500.0, 300.0),
            None,
            dupin_core::DisplaySelection::Main
        ),
        Some((0, false))
    );
}

#[test]
fn active_selection_uses_the_screen_with_the_largest_window_overlap() {
    let screens = [[0.0, 0.0, 1000.0, 800.0], [-1200.0, 100.0, 1200.0, 800.0]];
    let active = [-1000.0, 200.0, 800.0, 500.0];
    assert_eq!(
        select_screen(
            &screens,
            (400.0, 400.0),
            Some(active),
            dupin_core::DisplaySelection::Active
        ),
        Some((1, false))
    );
}

#[test]
fn active_selection_falls_back_to_primary_when_the_focused_window_is_unavailable() {
    let screens = [[0.0, 0.0, 1000.0, 800.0], [-1200.0, 100.0, 1200.0, 800.0]];
    assert_eq!(
        select_screen(
            &screens,
            (-500.0, 300.0),
            None,
            dupin_core::DisplaySelection::Active
        ),
        Some((0, true))
    );
    assert_eq!(
        select_screen(
            &screens,
            (-500.0, 300.0),
            Some([3000.0, 0.0, 400.0, 300.0]),
            dupin_core::DisplaySelection::Active,
        ),
        Some((0, true))
    );
    assert_eq!(
        select_screen(
            &screens,
            (-500.0, 300.0),
            Some([0.0, 0.0, 0.0, 300.0]),
            dupin_core::DisplaySelection::Active,
        ),
        Some((0, true))
    );
}

#[test]
fn active_window_coordinates_convert_to_appkit_points_with_nonzero_origins() {
    let frame = ax_to_appkit_frame(
        AxPoint { x: -1100.0, y: 150.0 },
        AxSize {
            width: 700.0,
            height: 450.0,
        },
        900.0,
    );
    assert_eq!(frame, [-1100.0, 300.0, 700.0, 450.0]);
    let screens = [[0.0, 0.0, 1440.0, 900.0], [-1280.0, 100.0, 1280.0, 800.0]];
    assert_eq!(screen_with_largest_overlap(&screens, frame), Some(1));
}

#[test]
fn no_displays_returns_no_selection() {
    assert_eq!(
        select_screen(&[], (0.0, 0.0), None, dupin_core::DisplaySelection::Main),
        None
    );
}
