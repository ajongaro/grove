//! The `?` key help: every key the dashboard handles, by where it works.
//! Keep in step with input.rs (and todo.rs for the to-do list).

pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Anywhere",
        &[
            ("C-g", "back to the dashboard"),
            ("C-q", "back from a finca or finca shell"),
            (
                "C-v",
                "in a finca: send the clipboard's file or image, paste its path",
            ),
        ],
    ),
    (
        "Dashboard",
        &[
            ("j k ↑ ↓", "move"),
            ("Tab h l", "switch pane"),
            ("j k", "also cross Sessions ↔ To-do at the ends"),
            ("t", "to-do list"),
            ("a", "add a project"),
            ("R", "update Grove (after cargo install)"),
            ("?", "this help"),
            ("q", "detach (sessions keep running)"),
        ],
    ),
    (
        "Projects",
        &[
            ("Enter", "start a session"),
            ("n", "start a named session"),
            ("s", "a shell in its folder (exit closes)"),
            ("e", "edit the project file"),
            ("c", "clear ended sessions"),
            ("d", "remove from Grove (folder kept)"),
        ],
    ),
    (
        "Sessions",
        &[
            ("Enter", "jump to its window"),
            ("r", "rename"),
            ("x", "kill"),
        ],
    ),
    (
        "To-do",
        &[
            ("j k gg G", "move"),
            ("o O", "add below / above"),
            ("i a A cc", "edit / replace text"),
            ("Enter Esc", "save (empty item is dropped)"),
            ("x space", "toggle done"),
            ("dd p P", "delete / paste below, above"),
            ("u", "undo"),
            ("E", "open in your editor"),
        ],
    ),
    (
        "Fincas",
        &[
            ("Enter", "connect to its Grove"),
            ("s", "open a shell there"),
            ("l Enter", "jump into a session or shell"),
            ("F", "add a finca (guided setup; a on a finca too)"),
        ],
    ),
];
