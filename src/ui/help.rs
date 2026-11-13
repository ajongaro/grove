//! The `?` key help: every key the dashboard handles, by where it works.
//! Keep in step with input.rs (and todo.rs for the to-do list).

pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Anywhere",
        &[
            ("C-g", "back to the dashboard"),
            ("C-\\", "shell under a session (again: swap)"),
            ("C-hjkl", "move between panes and desk columns"),
            ("C-q C-v", "finca: back / send the clipboard"),
        ],
    ),
    (
        "Dashboard",
        &[
            ("j k / J K", "move / by section (crosses at ends)"),
            ("Tab h l", "switch pane"),
            ("1–9", "jump to a pin"),
            ("t", "to-do list"),
            ("v V /", "session grid / pinned desk / find"),
            ("a", "add a project"),
            ("R T B", "update Grove / theme / top bar"),
            ("?", "this help"),
            ("q", "detach (sessions keep running)"),
        ],
    ),
    (
        "Projects",
        &[
            ("Enter", "start a session"),
            ("n", "start a named session"),
            ("s S", "shell: popup / stays (pins too)"),
            ("e", "edit the project file"),
            ("c C", "clear an ended session / all ended"),
            ("H d", "hide box / remove (folder kept)"),
        ],
    ),
    (
        "Sessions",
        &[
            ("Enter", "jump to it, or resume an ended one"),
            ("r", "rename (a shell too)"),
            ("x u U", "kill / restart / restart all"),
            ("p o", "pin or unpin / order the pins"),
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
            ("x c C", "toggle done / clear it / all done"),
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
            ("n u", "start a session there / restart one"),
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
