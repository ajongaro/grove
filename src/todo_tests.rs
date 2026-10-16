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
    let t = loaded("# Notes\n\n* [X] star item\nplain line\n");
    assert_eq!(texts(&t), ["# Notes", "x star item", "plain line"]);
    assert_eq!(
        t.serialize(),
        "- [ ] # Notes\n- [x] star item\n- [ ] plain line\n"
    );
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
    let mut t = loaded("- [x] old\n");
    keys(&mut t, "ccnew");
    press(&mut t, KeyCode::Enter);
    assert_eq!(texts(&t), ["x new"]);
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
    assert_eq!(keys(&mut t, "dd"), [Effect::Nothing, Effect::Changed]);
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
fn a_different_key_cancels_a_pending_d_or_g() {
    let mut t = loaded("- [ ] a\n- [ ] b\n");
    keys(&mut t, "djd");
    assert_eq!(texts(&t), ["a", "b"], "d j d is not dd");
    assert_eq!(t.idx, 1);
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
    keys(&mut t, "d");
    assert!(!t.at_top(), "a pending d must not let k leave the list");
    keys(&mut t, "jk");
    assert!(t.at_top());
    keys(&mut t, "j");
    assert!(!t.at_top());
    assert!(loaded("").at_top());
}

fn items(spec: &[(&str, bool)]) -> Vec<Item> {
    spec.iter()
        .map(|(t, d)| Item {
            text: t.to_string(),
            done: *d,
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
            Op::Add("d".into()),
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
    assert!(apply_ops(&[], &[Op::Add("  ".into())]).is_err());
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
    assert!(apply_ops(&start, &[Op::Add("x\n- [x] y".into())]).is_err());
    assert!(apply_ops(&start, &[Op::Reword(1, "x\ry".into())]).is_err());
}
