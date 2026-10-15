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
                "send the clipboard to a finca",
            ),
            ("prefix r", "reload Grove's tmux config"),
        ],
    ),
    (
        "Dashboard",
        &[
            ("j k ↑ ↓", "move"),
            ("Tab h l", "switch pane"),
            ("j k", "cross Sessions ↔ To-do at the ends"),
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
            ("s", "shell in its folder (C-d closes)"),
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
            ("C-u", "clear the line (while editing)"),
            ("h", "back to projects"),
        ],
    ),
    (
        "Fincas",
        &[
            ("Enter", "connect to its Grove"),
            ("s", "open a shell there"),
            ("l Enter", "jump into a session or shell"),
            ("F", "add a finca (or a on a finca)"),
        ],
    ),
    (
        "Prompts",
        &[
            ("Tab", "complete a path or SSH host"),
            ("j k", "pick a match from the list"),
            ("C-u", "clear the line"),
            ("Enter Esc", "confirm / cancel"),
        ],
    ),
];
