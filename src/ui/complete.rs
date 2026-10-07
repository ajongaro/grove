//! Shell-style Tab completion for directory paths in the add-project prompt.

/// Result of one Tab press.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// What the prompt should now contain.
    pub input: String,
    /// Full prompt values for each match when there's more than one (each
    /// ends in `/`); empty when the completion was unique or nothing matched.
    pub candidates: Vec<String>,
}

/// Completes the last path segment of `input` against the directories in its
/// parent. `~` expands; hidden folders only match a prefix starting with `.`.
/// Matching ignores case, like macOS's filesystem.
pub fn complete_path(input: &str) -> Completion {
    let unchanged = || Completion {
        input: input.to_string(),
        candidates: Vec::new(),
    };
    if input.is_empty() {
        return unchanged();
    }
    // A bare "~" means the home directory itself.
    let input_norm = if input == "~" {
        "~/".to_string()
    } else {
        input.to_string()
    };
    let (dir_part, prefix) = match input_norm.rfind('/') {
        Some(i) => input_norm.split_at(i + 1),
        None => ("", input_norm.as_str()),
    };
    let dir = if dir_part.is_empty() {
        std::path::PathBuf::from(".")
    } else {
        crate::config::expand_tilde(dir_part)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return unchanged();
    };

    let want = prefix.to_lowercase();
    let show_hidden = prefix.starts_with('.');
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| show_hidden || !n.starts_with('.'))
        .filter(|n| n.to_lowercase().starts_with(&want))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());

    match names.len() {
        0 => unchanged(),
        1 => Completion {
            input: format!("{dir_part}{}/", names[0]),
            candidates: Vec::new(),
        },
        _ => {
            let common = common_prefix(&names);
            // Never shrink what was typed (case-insensitive matches can share
            // less than the typed prefix in exact case).
            let completed = if common.chars().count() >= prefix.chars().count() {
                common
            } else {
                prefix.to_string()
            };
            Completion {
                input: format!("{dir_part}{completed}"),
                candidates: names.iter().map(|n| format!("{dir_part}{n}/")).collect(),
            }
        }
    }
}

/// Longest prefix shared by all names, compared case-insensitively and
/// returned in the first name's case.
fn common_prefix(names: &[String]) -> String {
    let first: Vec<char> = names[0].chars().collect();
    let mut len = first.len();
    for n in &names[1..] {
        len = len.min(
            first
                .iter()
                .zip(n.chars())
                .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                .count(),
        );
    }
    first[..len].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for p in [
            "streakwithfriends",
            "stream-tools",
            "grove",
            "grove-fi",
            ".hidden",
            "Archive",
        ] {
            std::fs::create_dir(d.path().join(p)).unwrap();
        }
        std::fs::write(d.path().join("stray-file.txt"), "x").unwrap();
        d
    }

    fn base(d: &tempfile::TempDir) -> String {
        format!("{}/", d.path().display())
    }

    #[test]
    fn unique_match_completes_with_slash() {
        let d = tree();
        let c = complete_path(&format!("{}streak", base(&d)));
        assert_eq!(c.input, format!("{}streakwithfriends/", base(&d)));
        assert!(c.candidates.is_empty());
    }

    #[test]
    fn several_matches_fill_common_prefix_and_list_them() {
        let d = tree();
        let c = complete_path(&format!("{}st", base(&d)));
        assert_eq!(c.input, format!("{}strea", base(&d)));
        assert_eq!(
            c.candidates,
            vec![
                format!("{}streakwithfriends/", base(&d)),
                format!("{}stream-tools/", base(&d))
            ]
        );
    }

    #[test]
    fn files_are_never_offered() {
        let d = tree();
        let c = complete_path(&format!("{}stray", base(&d)));
        assert_eq!(c.input, format!("{}stray", base(&d)));
        assert!(c.candidates.is_empty());
    }

    #[test]
    fn exact_dir_name_that_is_also_a_prefix_lists_both() {
        let d = tree();
        let c = complete_path(&format!("{}grove", base(&d)));
        assert_eq!(c.input, format!("{}grove", base(&d)));
        assert_eq!(
            c.candidates,
            vec![
                format!("{}grove/", base(&d)),
                format!("{}grove-fi/", base(&d))
            ]
        );
    }

    #[test]
    fn trailing_slash_lists_children_without_hidden() {
        let d = tree();
        let c = complete_path(&base(&d));
        let names: Vec<_> = c
            .candidates
            .iter()
            .map(|p| {
                p.trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "Archive",
                "grove",
                "grove-fi",
                "streakwithfriends",
                "stream-tools"
            ]
        );
    }

    #[test]
    fn dot_prefix_shows_hidden() {
        let d = tree();
        let c = complete_path(&format!("{}.h", base(&d)));
        assert_eq!(c.input, format!("{}.hidden/", base(&d)));
    }

    #[test]
    fn matching_ignores_case_and_uses_real_name() {
        let d = tree();
        let c = complete_path(&format!("{}arch", base(&d)));
        assert_eq!(c.input, format!("{}Archive/", base(&d)));
    }

    #[test]
    fn tilde_expands_but_is_kept_in_the_prompt() {
        // HOME always has at least one visible directory on a dev machine;
        // just check the prompt keeps the `~/` form.
        let c = complete_path("~/");
        assert!(c.candidates.iter().all(|p| p.starts_with("~/")), "{c:?}");
        let c = complete_path("~");
        assert!(c.input.starts_with("~/"), "{c:?}");
    }

    #[test]
    fn missing_parent_or_no_match_leaves_input_alone() {
        let d = tree();
        for input in ["/no/such/dir/x", &format!("{}zzz", base(&d)), ""] {
            let c = complete_path(input);
            assert_eq!(c.input, input);
            assert!(c.candidates.is_empty());
        }
    }
}
