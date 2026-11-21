use std::path::PathBuf;

use super::*;
use crate::ItemKind;

fn item(kind: ItemKind, name: &str) -> Item {
    Item::path(kind, PathBuf::from(format!("/x/{name}")), name.to_owned(), vec![])
}

fn searched(effects: &[Effect]) -> (Mode, u64, &str) {
    match effects {
        [
            Effect::Search {
                mode,
                generation,
                query,
            },
        ] => (*mode, *generation, query),
        other => panic!("expected one search, got {other:?}"),
    }
}

#[test]
fn starts_in_apps_mode_with_nothing_shown() {
    let s = Session::default();
    assert_eq!(s.mode(), Mode::Apps);
    assert_eq!(s.results(), Results::Idle);
    assert!(s.items().is_empty());
}

#[test]
fn typing_searches_apps() {
    let mut s = Session::default();
    let effects = s.set_query("saf");
    assert_eq!(searched(&effects).0, Mode::Apps);
    assert_eq!(searched(&effects).2, "saf");
}

#[test]
fn space_on_an_empty_query_enters_file_mode_and_is_consumed() {
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.mode(), Mode::Files);
    let effects = s.set_query("report");
    assert_eq!(searched(&effects).0, Mode::Files);
}

#[test]
fn space_inside_a_query_is_left_to_the_field() {
    let mut s = Session::default();
    s.set_query("visual");
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn space_in_file_mode_with_an_empty_query_is_swallowed() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.mode(), Mode::Files);
}

#[test]
fn backspace_on_an_empty_query_leaves_file_mode() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    s.set_query("a");
    // Deleting the last character is the field's job...
    assert_eq!(s.intent(Intent::Backspace), None);
    s.set_query("");
    // ...and the next Backspace, on an empty query, leaves the mode.
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn backspace_on_an_empty_apps_query_is_left_to_the_field() {
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Backspace), None);
}

#[test]
fn results_replace_on_first_batch_and_append_after() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    assert!(s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Safari")]
    }));
    assert!(s.deliver(Delivery::Done { generation: g1 }));
    let g2 = searched(&s.set_query("sa")).1;
    // The old rows stay until the new search answers.
    assert_eq!(s.items().len(), 1);
    assert_eq!(s.results(), Results::Searching { replaced: false });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item(ItemKind::Application, "Safari 2")],
    });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item(ItemKind::Application, "Safari 3")],
    });
    let titles: Vec<_> = s.items().iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Safari 2", "Safari 3"]);
}

#[test]
fn stale_answers_are_dropped() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    let _g2 = searched(&s.set_query("sa")).1;
    assert!(!s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Old")]
    }));
    assert!(s.items().is_empty());
}

#[test]
fn done_without_batches_clears_old_rows() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    let g2 = searched(&s.set_query("sxq")).1;
    s.deliver(Delivery::Done { generation: g2 });
    assert!(s.items().is_empty());
    assert_eq!(s.results(), Results::Done);
}

#[test]
fn clearing_the_query_cancels_and_ignores_late_answers() {
    let mut s = Session::default();
    let g = searched(&s.set_query("s")).1;
    assert_eq!(s.set_query(""), vec![Effect::CancelSearch]);
    assert!(!s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Late")]
    }));
    assert_eq!(s.results(), Results::Idle);
}

#[test]
fn switching_modes_never_shows_the_other_modes_rows() {
    let mut s = Session::default();
    let g = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    s.set_query("");
    s.intent(Intent::Space);
    s.set_query("s");
    assert!(s.items().is_empty());
}

#[test]
fn arrows_wrap_and_reveal_the_selection() {
    let mut s = Session::default();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(|n| item(ItemKind::Application, n)).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    assert_eq!(s.intent(Intent::Up), Some(vec![Effect::RevealSelection]));
    assert_eq!(s.selected(), 2);
    s.intent(Intent::Down);
    assert_eq!(s.selected(), 0);
}

#[test]
fn enter_runs_the_primary_action_then_hides_and_resets() {
    let mut s = Session::default();
    let g = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    s.deliver(Delivery::Done { generation: g });
    let effects = s.intent(Intent::Open).unwrap();
    assert_eq!(
        effects,
        vec![Effect::Run(Action::Open("/x/Safari".into())), Effect::Hide]
    );
    assert_eq!(s.query(), "");
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn command_enter_reveals_in_the_file_manager() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    let g = searched(&s.set_query("notes")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::File, "notes.md")],
    });
    let effects = s.intent(Intent::OpenAlternate).unwrap();
    assert_eq!(effects[0], Effect::Run(Action::Reveal("/x/notes.md".into())));
    assert!(
        effects.contains(&Effect::CancelSearch),
        "the unfinished search stops: {effects:?}"
    );
    assert_eq!(effects.last(), Some(&Effect::Hide));
}

#[test]
fn enter_with_no_results_does_nothing() {
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Open), Some(vec![]));
}

#[test]
fn escape_hides_and_resets_file_mode() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Dismiss), Some(vec![Effect::Hide]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn a_click_selects_by_id_even_after_rows_move() {
    let mut s = Session::default();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(|n| item(ItemKind::Application, n)).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    assert!(s.select_id("/x/C"));
    assert_eq!(s.selected_item().unwrap().title, "C");
    assert!(!s.select_id("/x/gone"));
    assert_eq!(s.selected(), 2);
}
