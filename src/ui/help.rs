//! The `?` key help: every key the dashboard handles, by where it works.
//! Keep in step with input.rs (and todo.rs for the to-do list).

pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Anywhere",
        &[
            ("C-g", "back to the dashboard"),
            ("C-q", "back from a finca or finca shell"),
            ("C-v", "send the clipboard to a finca"),
            ("prefix r", "reload Grove's tmux config"),
        ],
    ),
    (
        "Dashboard",
        &[
            ("j k ↑ ↓", "move (crosses panes at the ends)"),
            ("Tab h l", "switch pane"),
            ("1–9", "jump to a pin"),
            ("t", "to-do list"),
            ("v /", "session grid / find a session"),
            ("a", "add a project"),
            ("R", "update Grove (after upgrading)"),
            ("?", "this help"),
            ("q", "detach (sessions keep running)"),
        ],
    ),
    (
        "Projects",
        &[
            ("Enter", "start a session"),
            ("n", "start a named session"),
            ("s S", "shell: quick popup / stays open"),
            ("e", "edit the project file"),
            ("c", "clear ended sessions"),
            ("d", "remove from Grove (folder kept)"),
        ],
    ),
    (
        "Sessions",
        &[
            ("Enter", "jump to it, or resume an ended one"),
            ("r", "rename (a shell too)"),
            ("x", "kill, or close a shell"),
            ("p", "pin / unpin (any account or finca)"),
        ],
    ),
    (
        "Grid",
        &[
            ("hjkl Tab", "move / next waiting (S-Tab back)"),
            ("Enter", "jump to its window"),
            ("i", "reply to a waiting session"),
            ("x", "kill it (asks first)"),
            ("v Esc", "back to the dashboard"),
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
            ("s", "open a shell there (r renames it)"),
            ("Enter", "connect, or jump into a session"),
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
