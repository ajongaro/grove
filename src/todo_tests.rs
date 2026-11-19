use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn keys(t: &mut Todos, s: &str) -> Vec<Effect> {
    s.chars()
        .map(|c| t.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)))
        .collect()
}
fn press(t: &mut Todos, code: KeyCode) -> Effect {
    t.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn loaded(text: &str) -> Todos {
    let mut t = Todos::default();
    t.load(text);
    t
}
fn texts(t: &Todos) -> Vec<String> {
    t.items
        .iter()
        .map(|i| format!("{}{}", if i.done { "x " } else { "" }, i.text))
        .collect()
}

// ---------- file format ----------

#[test]
fn parses_and_writes_a_markdown_checklist() {
    let text = "- [ ] buy milk\n- [x] ship grove\n";
    let t = loaded(text);
    assert_eq!(texts(&t), ["buy milk", "x ship grove"]);
    assert_eq!(t.serialize(), text);
}

#[test]
fn other_lines_become_open_items_and_blank_lines_are_dropped() {
    // A heading isn't an item: it names a band (`# Notes` is normal).
    let t = loaded("# Notes\n\n* [X] star item\nplain line\n");
    assert_eq!(texts(&t), ["x star item", "plain line"]);
    assert_eq!(t.serialize(), "- [x] star item\n- [ ] plain line\n");
}

// ---------- bands and due dates ----------

fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// `h:`/`l:` for high and low items, as the list holds them.
fn banded(t: &Todos) -> Vec<String> {
    t.items
        .iter()
        .map(|i| {
            let band = match i.priority {
                Priority::High => "h:",
                Priority::Normal => "",
                Priority::Low => "l:",
            };
            format!("{band}{}", i.text)
        })
        .collect()
}

const BANDED: &str =
    "## High priority\n- [ ] h\n\n## Normal\n- [ ] a\n- [x] b\n\n## Low priority\n- [ ] l\n";

#[test]
fn bands_are_read_from_headings_and_written_back() {
    let t = loaded(BANDED);
    assert_eq!(banded(&t), ["h:h", "a", "b", "l:l"]);
    assert_eq!(t.serialize(), BANDED);
    // An empty band has no heading.
    let t = loaded("## High priority\n- [ ] h\n## Normal\n- [ ] a\n");
    assert_eq!(
        t.serialize(),
        "## High priority\n- [ ] h\n\n## Normal\n- [ ] a\n"
    );
}

#[test]
fn a_list_with_no_bands_is_written_as_before() {
    let text = "- [ ] a\n- [x] b\n";
    assert_eq!(loaded(text).serialize(), text);
    assert_eq!(loaded("## Normal\n- [ ] a\n").serialize(), "- [ ] a\n");
}

#[test]
fn headings_in_any_order_or_case_group_the_items() {
    let t = loaded("- [ ] n0\n## low stuff\n- [ ] l\n# HIGH\n- [ ] h\n## Notes\n- [ ] n1\n");
    assert_eq!(banded(&t), ["h:h", "n0", "n1", "l:l"]);
}

#[test]
fn a_due_suffix_becomes_the_due_date() {
    let t = loaded("- [ ] pay (due 2026-12-01)\n- [ ] ask (due soon)\n");
    assert_eq!(texts(&t), ["pay", "ask (due soon)"]);
    assert_eq!(t.items[0].due, Some(day(2026, 12, 1)));
    assert_eq!(t.items[1].due, None);
    assert_eq!(
        t.serialize(),
        "- [ ] pay (due 2026-12-01)\n- [ ] ask (due soon)\n"
    );
}

#[test]
fn plus_and_minus_move_an_item_between_bands() {
    let mut t = loaded("## High\n- [ ] h\n## Normal\n- [ ] a\n- [ ] b\n## Low\n- [ ] l\n");
    keys(&mut t, "jj");
    assert_eq!(keys(&mut t, "+"), [Effect::Changed]);
    // To the end of the band above, the cursor with it.
    assert_eq!(banded(&t), ["h:h", "h:b", "a", "l:l"]);
    assert_eq!(t.items[t.idx].text, "b");
    assert_eq!(keys(&mut t, "+"), [Effect::Nothing], "already high");
    keys(&mut t, "--");
    // To the start of the band below.
    assert_eq!(banded(&t), ["h:h", "a", "l:b", "l:l"]);
    assert_eq!(t.items[t.idx].text, "b");
    assert_eq!(keys(&mut t, "-"), [Effect::Nothing], "already low");
    keys(&mut t, "u");
    assert_eq!(banded(&t), ["h:h", "b", "a", "l:l"]);
    assert_eq!(
        keys(&mut loaded(""), "+-"),
        [Effect::Nothing, Effect::Nothing]
    );
}

#[test]
fn new_items_and_pastes_take_the_cursors_band() {
    let mut t = loaded("## High\n- [ ] h\n## Low\n- [ ] l\n");
    keys(&mut t, "j");
    keys(&mut t, "nx");
    press(&mut t, KeyCode::Enter);
    assert_eq!(banded(&t), ["h:h", "l:l", "l:x"]);
    // d on the high item, p among the low ones: it's low now.
    keys(&mut t, "gg");
    keys(&mut t, "d");
    keys(&mut t, "p");
    assert_eq!(banded(&t), ["l:l", "l:h", "l:x"]);
    let mut empty = loaded("");
    keys(&mut empty, "nz");
    press(&mut empty, KeyCode::Enter);
    assert_eq!(banded(&empty), ["z"]);
}

#[test]
fn at_asks_for_a_due_date_and_set_due_saves_it() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    keys(&mut t, "j");
    assert_eq!(keys(&mut t, "@"), [Effect::AskDue]);
    assert_eq!(t.set_due(Some(day(2026, 12, 1))), Effect::Changed);
    assert_eq!(t.items[1].due, Some(day(2026, 12, 1)));
    assert_eq!(t.set_due(Some(day(2026, 12, 1))), Effect::Nothing);
    keys(&mut t, "u");
    assert_eq!(t.items[1].due, None);
    assert_eq!(keys(&mut loaded(""), "@"), [Effect::Nothing]);
}

#[test]
fn empty_list_serializes_to_nothing() {
    assert_eq!(loaded("").serialize(), "");
}

// ---------- moving ----------

#[test]
fn j_k_gg_and_g_move_within_bounds() {
    let mut t = loaded("- [ ] a\n- [ ] b\n- [ ] c\n");
    keys(&mut t, "jjj");
    assert_eq!(t.idx, 2);
    keys(&mut t, "k");
    assert_eq!(t.idx, 1);
    keys(&mut t, "gg");
    assert_eq!(t.idx, 0);
    keys(&mut t, "G");
    assert_eq!(t.idx, 2);
}

// ---------- adding and editing ----------

#[test]
fn o_adds_below_and_enter_saves() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    assert_eq!(keys(&mut t, "o"), [Effect::Nothing]);
    assert!(t.inserting());
    keys(&mut t, "new");
    assert_eq!(press(&mut t, KeyCode::Enter), Effect::Changed);
    assert!(!t.inserting());
    assert_eq!(texts(&t), ["a", "new", "b"]);
    assert_eq!(t.idx, 1);
}

#[test]
fn big_o_adds_above_and_esc_also_saves() {
    let mut t = loaded("- [ ] a\n");
    keys(&mut t, "Ohi");
    assert_eq!(press(&mut t, KeyCode::Esc), Effect::Changed);
    assert_eq!(texts(&t), ["hi", "a"]);
    assert_eq!(t.idx, 0);
}

#[test]
fn o_on_an_empty_list_adds_the_first_item() {
    let mut t = loaded("");
    keys(&mut t, "ofirst");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["first"]);
}

#[test]
fn a_new_item_left_empty_is_discarded_without_saving() {
    let mut t = loaded("- [ ] a\n");
    keys(&mut t, "o");
    assert_eq!(press(&mut t, KeyCode::Esc), Effect::Nothing);
    assert_eq!(texts(&t), ["a"]);
    assert_eq!(t.idx, 0);
    // Nothing happened, so there's nothing to undo.
    assert_eq!(keys(&mut t, "u"), [Effect::Nothing]);
}

#[test]
fn i_and_a_append_to_the_item_and_backspace_deletes() {
    let mut t = loaded("- [ ] milk\n");
    keys(&mut t, "i");
    press(&mut t, KeyCode::Backspace);
    keys(&mut t, "nt");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["milnt"]);
    keys(&mut t, "A!");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["milnt!"]);
}

#[test]
fn cc_replaces_the_text() {
    let mut t = loaded("- [ ] old\n");
    keys(&mut t, "ccnew");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["new"]);
}

#[test]
fn editing_an_item_to_nothing_removes_it() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    keys(&mut t, "cc");
    assert_eq!(press(&mut t, KeyCode::Enter), Effect::Changed);
    assert_eq!(texts(&t), ["b"]);
}

#[test]
fn ctrl_u_clears_the_line_while_inserting() {
    let mut t = loaded("- [ ] long text\n");
    keys(&mut t, "A");
    t.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    keys(&mut t, "short");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["short"]);
}

#[test]
fn typing_q_or_x_while_inserting_is_just_text() {
    let mut t = loaded("");
    keys(&mut t, "oqx dd");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["qx dd"]);
}

// ---------- done, delete, paste, undo ----------

#[test]
fn x_and_space_toggle_done() {
    let mut t = loaded("- [ ] a\n");
    assert_eq!(keys(&mut t, "x"), [Effect::Changed]);
    assert_eq!(texts(&t), ["x a"]);
    assert_eq!(press(&mut t, KeyCode::Char(' ')), Effect::Changed);
    assert_eq!(texts(&t), ["a"]);
}

#[test]
fn dd_deletes_into_the_register_and_p_pastes_below() {
    let mut t = loaded("- [ ] a\n- [x] b\n- [ ] c\n");
    keys(&mut t, "j");
    // d deletes at once; vim's dd is the same single delete.
    assert_eq!(keys(&mut t, "dd"), [Effect::Changed, Effect::Nothing]);
    assert_eq!(texts(&t), ["a", "c"]);
    assert_eq!(t.idx, 1);
    assert_eq!(keys(&mut t, "p"), [Effect::Changed]);
    assert_eq!(texts(&t), ["a", "c", "x b"]);
    assert_eq!(t.idx, 2);
    keys(&mut t, "ggP");
    assert_eq!(texts(&t), ["x b", "a", "c", "x b"]);
    assert_eq!(t.idx, 0);
}

#[test]
fn dd_on_the_last_item_moves_up() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    keys(&mut t, "Gdd");
    assert_eq!(texts(&t), ["a"]);
    assert_eq!(t.idx, 0);
}

#[test]
fn p_with_nothing_deleted_does_nothing() {
    let mut t = loaded("- [ ] a\n");
    assert_eq!(keys(&mut t, "p"), [Effect::Nothing]);
}

#[test]
fn u_undoes_the_last_change() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    keys(&mut t, "dd");
    assert_eq!(keys(&mut t, "u"), [Effect::Changed]);
    assert_eq!(texts(&t), ["a", "b"]);
    keys(&mut t, "jx");
    keys(&mut t, "u");
    assert_eq!(texts(&t), ["a", "b"]);
    keys(&mut t, "ofresh");
    press(&mut t, KeyCode::Enter);
    keys(&mut t, "u");
    assert_eq!(texts(&t), ["a", "b"]);
}

#[test]
fn a_different_key_cancels_a_pending_g_and_d_deletes_each_time() {
    let mut t = loaded("- [ ] a\n- [ ] b\n- [ ] c\n");
    keys(&mut t, "gjg");
    assert_eq!(t.idx, 1, "g j g is not gg");
    // d, then another key, then d: two deletes.
    keys(&mut t, "dkd");
    assert_eq!(texts(&t), ["c"]);
}

#[test]
fn ctrl_keys_do_nothing_in_normal_mode() {
    // Raw mode delivers Ctrl-C as 'c': twice must not be `cc`.
    let mut t = loaded("- [ ] a\n");
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    t.key(ctrl_c);
    t.key(ctrl_c);
    assert!(!t.inserting());
    assert_eq!(texts(&t), ["a"]);
}

#[test]
fn big_e_asks_for_the_editor() {
    let mut t = loaded("");
    assert_eq!(keys(&mut t, "E"), [Effect::OpenEditor]);
}

// ---------- reloading ----------

#[test]
fn load_keeps_the_cursor_and_register_but_an_outside_edit_clears_undo() {
    let mut t = loaded("- [ ] a\n- [ ] b\n- [ ] c\n");
    keys(&mut t, "jdd");
    // Same content again: nothing changes.
    t.load(&t.serialize());
    assert_eq!(
        keys(&mut t, "u"),
        [Effect::Changed],
        "undo survives a no-op load"
    );
    keys(&mut t, "Gdd");
    t.load("- [ ] edited elsewhere\n");
    assert_eq!(texts(&t), ["edited elsewhere"]);
    assert_eq!(t.idx, 0, "clamped");
    assert_eq!(
        keys(&mut t, "u"),
        [Effect::Nothing],
        "undo would revert the outside edit"
    );
    assert_eq!(keys(&mut t, "p"), [Effect::Changed], "register kept");
}

#[test]
fn reset_empties_the_list_but_keeps_the_register() {
    // Switching projects: dd in one, p in another moves the item.
    let mut t = loaded("- [ ] carry me\n");
    keys(&mut t, "dd");
    t.reset();
    t.load("- [ ] other\n");
    assert_eq!(
        keys(&mut t, "u"),
        [Effect::Nothing],
        "undo belonged to the other list"
    );
    keys(&mut t, "p");
    assert_eq!(texts(&t), ["other", "carry me"]);
}

#[test]
fn open_count_counts_unchecked_items() {
    assert_eq!(open_count("- [ ] a\n- [x] b\nplain\n\n"), 2);
    assert_eq!(open_count(""), 0);
}

#[test]
fn at_top_only_on_the_first_item_with_nothing_half_typed() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    assert!(t.at_top());
    keys(&mut t, "g");
    assert!(!t.at_top(), "a pending g must not let k leave the list");
    keys(&mut t, "jk");
    assert!(t.at_top());
    keys(&mut t, "j");
    assert!(!t.at_top());
    assert!(loaded("").at_top());
}

fn items(spec: &[(&str, bool)]) -> Vec<Item> {
    spec.iter()
        .map(|(t, d)| Item {
            done: *d,
            ..Item::new(*t)
        })
        .collect()
}

#[test]
fn ops_number_the_original_list() {
    let start = items(&[("a", false), ("b", false), ("c", false)]);
    let out = apply_ops(
        &start,
        &[
            Op::Remove(1),
            Op::Check(3),
            Op::Reword(2, "B".into()),
            Op::add("d"),
        ],
    )
    .unwrap();
    assert_eq!(out, items(&[("B", false), ("c", true), ("d", false)]));
}

#[test]
fn uncheck_and_out_of_range() {
    let start = items(&[("a", true)]);
    assert_eq!(
        apply_ops(&start, &[Op::Uncheck(1)]).unwrap(),
        items(&[("a", false)])
    );
    let e = apply_ops(&start, &[Op::Check(2)]).unwrap_err();
    assert!(e.contains('2') && e.contains("1 item"), "{e}");
    assert!(apply_ops(&start, &[Op::Check(0)]).is_err());
}

#[test]
fn an_op_on_a_removed_item_is_an_error() {
    let start = items(&[("a", false)]);
    assert!(apply_ops(&start, &[Op::Remove(1), Op::Check(1)]).is_err());
}

#[test]
fn empty_text_is_refused() {
    assert!(apply_ops(&[], &[Op::add("  ")]).is_err());
}

#[test]
fn serialize_items_matches_the_dashboard_format() {
    let it = items(&[("a", false), ("b", true)]);
    assert_eq!(serialize_items(&it), "- [ ] a\n- [x] b\n");
    assert_eq!(parse(&serialize_items(&it)), it);
}

#[test]
fn text_with_a_line_break_is_refused() {
    let start = items(&[("a", false)]);
    assert!(apply_ops(&start, &[Op::add("x\n- [x] y")]).is_err());
    assert!(apply_ops(&start, &[Op::Reword(1, "x\ry".into())]).is_err());
}

#[test]
fn a_i_and_cc_in_an_empty_list_start_the_first_item() {
    for start in ["a", "A", "i", "cc"] {
        let mut t = loaded("");
        keys(&mut t, start);
        assert!(t.inserting(), "{start}");
        keys(&mut t, "buy milk");
        assert_eq!(press(&mut t, KeyCode::Enter), Effect::Changed, "{start}");
        assert_eq!(t.serialize(), "- [ ] buy milk\n", "{start}");
        // Nothing typed: nothing added.
        let mut t = loaded("");
        keys(&mut t, start);
        assert_eq!(press(&mut t, KeyCode::Esc), Effect::Nothing, "{start}");
        assert!(t.items.is_empty(), "{start}");
    }
}

// ---------- clearing done items ----------

#[test]
fn c_clears_a_done_item_and_hands_it_over_for_the_archive() {
    let mut t = loaded("- [ ] a\n- [x] b\n- [ ] c\n");
    t.idx = 1;
    assert_eq!(keys(&mut t, "c"), [Effect::Changed]);
    assert_eq!(texts(&t), ["a", "c"]);
    assert_eq!(t.idx, 1, "the cursor moves on to the next item");
    let cleared = t.take_cleared();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].text, "b");
    assert!(t.take_cleared().is_empty(), "handed over once");
    // u brings it back.
    keys(&mut t, "u");
    assert_eq!(texts(&t), ["a", "x b", "c"]);
}

#[test]
fn c_on_an_open_item_still_starts_cc() {
    let mut t = loaded("- [ ] a\n- [x] b\n");
    keys(&mut t, "ccnew");
    press(&mut t, KeyCode::Esc);
    assert_eq!(texts(&t), ["new", "x b"]);
    assert!(t.take_cleared().is_empty());
}

#[test]
fn shift_c_clears_every_done_item_keeping_the_cursor_on_its_item() {
    let mut t = loaded("- [x] a\n- [ ] b\n- [x] c\n- [ ] d\n- [x] e\n");
    t.idx = 3; // d
    assert_eq!(keys(&mut t, "C"), [Effect::Changed]);
    assert_eq!(texts(&t), ["b", "d"]);
    assert_eq!(t.items[t.idx].text, "d");
    let cleared: Vec<String> = t.take_cleared().into_iter().map(|i| i.text).collect();
    assert_eq!(cleared, ["a", "c", "e"]);
    // Nothing done: nothing happens.
    assert_eq!(keys(&mut t, "C"), [Effect::Nothing]);
}

// ---------- the keys the other panes use ----------

#[test]
fn n_adds_r_edits_enter_checks_off_and_e_opens_the_editor() {
    let mut t = loaded("- [ ] a\n");
    keys(&mut t, "nb");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["a", "b"]);
    keys(&mut t, "r!");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["a", "b!"], "r edits the text, keeping it");
    assert_eq!(press(&mut t, KeyCode::Enter), Effect::Changed);
    assert_eq!(texts(&t), ["a", "x b!"]);
    assert_eq!(keys(&mut t, "e"), [Effect::OpenEditor]);
}

#[test]
fn ops_set_priority_and_due_and_keep_bands() {
    let start = parse("## High\n- [ ] h\n## Normal\n- [ ] a\n- [ ] b\n");
    let out = apply_ops(
        &start,
        &[
            Op::Add {
                text: "h2".into(),
                priority: Some(Priority::High),
                due: Some(Some(day(2026, 12, 1))),
            },
            Op::Set {
                n: 3,
                priority: Some(Priority::Low),
                due: None,
            },
        ],
    )
    .unwrap();
    let got: Vec<(&str, Priority)> = out.iter().map(|i| (i.text.as_str(), i.priority)).collect();
    // An add lands at the end of its band; a set moves the item.
    assert_eq!(
        got,
        [
            ("h", Priority::High),
            ("h2", Priority::High),
            ("a", Priority::Normal),
            ("b", Priority::Low),
        ]
    );
    assert_eq!(out[1].due, Some(day(2026, 12, 1)));
    let cleared = apply_ops(
        &out,
        &[Op::Set {
            n: 2,
            priority: None,
            due: Some(None),
        }],
    )
    .unwrap();
    assert_eq!(cleared[1].due, None);
    assert_eq!(cleared[1].priority, Priority::High, "untouched");
}
