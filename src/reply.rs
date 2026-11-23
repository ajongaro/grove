//! What a session last did, for the Pinned box (`m`): the opening of
//! Claude's last reply, from its Stop hook, as plain text.

/// How much of a reply is kept.
const MAX: usize = 300;

/// The first paragraph of `reply` with words in it, Markdown marks taken
/// out and lines joined; None when there's none. A paragraph that is only a
/// heading is passed over, and so is code.
pub fn summary(reply: &str) -> Option<String> {
    let mut in_code = false;
    let mut para: Vec<String> = Vec::new();
    let mut found = None;
    for raw in reply.lines().chain(std::iter::once("")) {
        let line = raw.trim();
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if line.is_empty() {
            let only_heading = para.len() == 1 && reply_heading(&para[0]);
            if !para.is_empty() && !only_heading {
                found = Some(para.join(" "));
                break;
            }
            para.clear();
            continue;
        }
        // Rules and table rows say nothing on their own.
        if line
            .chars()
            .all(|c| matches!(c, '-' | '*' | '_' | '=' | ' '))
            || line.starts_with('|')
        {
            continue;
        }
        para.push(line.to_string());
    }
    let text = plain(&found?);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > MAX {
        let cut: String = text.chars().take(MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        text
    })
}

fn reply_heading(line: &str) -> bool {
    line.starts_with('#')
}

/// A line's text without its Markdown: heading, list and quote marks in
/// front, and emphasis and code marks within.
fn plain(text: &str) -> String {
    let mut s = text.trim_start_matches(['#', '>', ' ']).to_string();
    for mark in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(mark) {
            s = rest.to_string();
        }
    }
    if let Some((n, rest)) = s.split_once(". ")
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
    {
        s = rest.to_string();
    }
    // Marks within, and those at the start of each joined line.
    let s = s
        .replace(" - ", " ")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "");
    s.split(' ')
        .filter(|w| !matches!(*w, "-" | "*" | "+" | ">" | "#" | "##" | "###"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_paragraph_as_plain_text() {
        let r = "Pinned finca shells now **connect** over `ssh`.\nAll 724 tests pass.\n\nDetails:\n- one\n- two";
        assert_eq!(
            summary(r).as_deref(),
            Some("Pinned finca shells now connect over ssh. All 724 tests pass.")
        );
    }

    #[test]
    fn headings_rules_code_and_lists_are_passed_over_or_cleaned() {
        let r =
            "## Summary\n\n---\n\n```rust\nfn x() {}\n```\n\n- **Fixed** the rename\n- Added `X`";
        assert_eq!(summary(r).as_deref(), Some("Fixed the rename Added X"));
        assert_eq!(
            summary("1. First step done").as_deref(),
            Some("First step done")
        );
        assert_eq!(summary("> quoted").as_deref(), Some("quoted"));
    }

    #[test]
    fn nothing_to_say() {
        assert_eq!(summary(""), None);
        assert_eq!(summary("   \n\n"), None);
        assert_eq!(summary("```\ncode only\n```"), None);
    }

    #[test]
    fn long_replies_are_cut() {
        let r = "word ".repeat(200);
        let s = summary(&r).unwrap();
        assert!(s.chars().count() <= MAX);
        assert!(s.ends_with('…'));
    }
}
