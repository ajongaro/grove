//! The session grid (`v`): every live session as a tile showing the bottom
//! of its screen. Layout and movement are pure; drawing is at the bottom.

// Wired into the dashboard in plan tasks 7-9; this goes then.
#![allow(dead_code)]

use chrono::{DateTime, Utc};

use super::app::App;
use crate::model::State;

/// Seconds without a good fetch before a finca's tiles go stale.
pub const STALE_SECS: i64 = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// The finca's (slug, name), or None for this machine.
    pub finca: Option<(String, String)>,
    /// The finca destination that last answered.
    pub via: Option<String>,
    pub project: String,
    pub name: String,
    /// None for a finca tile from the status poll (its Grove is too old to
    /// reply to).
    pub session_id: Option<String>,
    pub state: State,
    pub updated_at: DateTime<Utc>,
    pub window: Option<String>,
    /// None: no live screen for this tile.
    pub screen: Option<String>,
    pub stale: bool,
}

impl Tile {
    pub fn handle(&self) -> String {
        format!("{}:{}", self.project, self.name)
    }
}

/// Every live session: this machine's (projects in pane order, oldest
/// first), then each finca's in the Fincas pane order.
pub fn tiles(app: &App, now: DateTime<Utc>) -> Vec<Tile> {
    let mut out = Vec::new();
    for p in &app.projects {
        for s in app
            .sessions
            .iter()
            .filter(|s| s.project == p.slug && s.state != State::Ended)
        {
            out.push(Tile {
                finca: None,
                via: None,
                project: s.project.clone(),
                name: s.name.clone(),
                session_id: Some(s.id.clone()),
                state: s.state,
                updated_at: s.updated_at,
                window: s.tmux_window.clone(),
                screen: Some(app.local_screens.get(&s.id).cloned().unwrap_or_default()),
                stale: false,
            });
        }
    }
    for f in &app.fincas {
        let finca = Some((f.slug.clone(), f.name.clone()));
        let screens = app
            .finca_screens
            .get(&f.slug)
            .filter(|_| !app.too_old.contains(&f.slug));
        if let Some((at, screens)) = screens {
            let stale = (now - *at).num_seconds() > STALE_SECS;
            for s in &screens.sessions {
                out.push(Tile {
                    finca: finca.clone(),
                    via: f.via.clone(),
                    project: s.project.clone(),
                    name: s.name.clone(),
                    session_id: Some(s.id.clone()),
                    state: s.state,
                    updated_at: s.updated_at,
                    window: s.window.clone(),
                    screen: Some(s.screen.clone()),
                    stale,
                });
            }
        } else if let Some(st) = &f.last_status {
            for p in &st.projects {
                for s in p.sessions.iter().filter(|s| s.state != State::Ended) {
                    out.push(Tile {
                        finca: finca.clone(),
                        via: f.via.clone(),
                        project: p.slug.clone(),
                        name: s.name.clone(),
                        session_id: None,
                        state: s.state,
                        updated_at: s.updated_at,
                        window: s.window.clone(),
                        screen: None,
                        stale: false,
                    });
                }
            }
        }
    }
    out
}

/// The smallest tile worth drawing (border included).
pub const MIN_W: u16 = 40;
pub const MIN_H: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub cols: usize,
    /// Rows on screen (more may be scrolled to).
    pub rows: usize,
    pub tile_w: u16,
    pub tile_h: u16,
}

/// Tiles of at least MIN_W × MIN_H, filling `width` × `height`; extra rows
/// scroll rather than shrink.
pub fn layout(n: usize, width: u16, height: u16) -> Layout {
    let cols = usize::from(width / MIN_W).clamp(1, n.max(1));
    let rows = n
        .div_ceil(cols)
        .clamp(1, usize::from(height / MIN_H).max(1));
    Layout {
        cols,
        rows,
        tile_w: width / cols as u16,
        tile_h: height / rows as u16,
    }
}

/// The top visible row that keeps `sel` on screen.
pub fn first_row(sel: usize, cols: usize, rows: usize) -> usize {
    (sel / cols.max(1)).saturating_sub(rows.saturating_sub(1))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The neighbour of `sel` in `dir`, staying put at an edge.
pub fn step(sel: usize, n: usize, cols: usize, dir: Dir) -> usize {
    if n == 0 {
        return 0;
    }
    let cols = cols.max(1);
    let sel = sel.min(n - 1);
    let col = sel % cols;
    match dir {
        Dir::Left if col > 0 => sel - 1,
        Dir::Right if col + 1 < cols && sel + 1 < n => sel + 1,
        Dir::Up if sel >= cols => sel - cols,
        // Onto a short last row: its last tile.
        Dir::Down if (sel / cols + 1) * cols < n => (sel + cols).min(n - 1),
        _ => sel,
    }
}

/// The next tile (wrapping) that needs you or is your turn; `from` if none.
pub fn next_waiting(states: &[State], from: usize, forward: bool) -> usize {
    let n = states.len();
    (1..n)
        .map(|k| {
            if forward {
                (from + k) % n
            } else {
                (from + n - k) % n
            }
        })
        .find(|&i| matches!(states[i], State::NeedsYou | State::YourTurn))
        .unwrap_or(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn layout_fills_wide_and_tall_screens() {
        assert_eq!(
            layout(4, 110, 28),
            Layout {
                cols: 2,
                rows: 2,
                tile_w: 55,
                tile_h: 14
            }
        );
        assert_eq!(
            layout(1, 200, 60),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 200,
                tile_h: 60
            }
        );
        // 200 wide fits 5 columns; 7 tiles is 2 rows.
        assert_eq!(
            layout(7, 200, 60),
            Layout {
                cols: 5,
                rows: 2,
                tile_w: 40,
                tile_h: 30
            }
        );
    }

    #[test]
    fn layout_scrolls_instead_of_shrinking() {
        // 80x24: 2 columns, 3 rows of 8; 10 tiles need 5 rows.
        let l = layout(10, 80, 24);
        assert_eq!((l.cols, l.rows, l.tile_h), (2, 3, 8));
    }

    #[test]
    fn tiny_terminals_get_one_clipped_tile() {
        assert_eq!(
            layout(3, 30, 6),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 30,
                tile_h: 6
            }
        );
        assert_eq!(
            layout(0, 0, 0),
            Layout {
                cols: 1,
                rows: 1,
                tile_w: 0,
                tile_h: 0
            }
        );
    }

    #[test]
    fn first_row_keeps_the_selection_visible() {
        assert_eq!(first_row(0, 2, 3), 0);
        assert_eq!(first_row(5, 2, 3), 0); // row 2
        assert_eq!(first_row(6, 2, 3), 1); // row 3
        assert_eq!(first_row(9, 2, 3), 2); // row 4
        assert_eq!(first_row(4, 2, 0), 2); // no visible rows: no panic
    }

    #[test]
    fn step_moves_within_the_grid() {
        // 5 tiles, 3 columns: [0 1 2] [3 4]
        assert_eq!(step(0, 5, 3, Dir::Left), 0);
        assert_eq!(step(1, 5, 3, Dir::Right), 2);
        assert_eq!(step(2, 5, 3, Dir::Right), 2);
        assert_eq!(step(4, 5, 3, Dir::Right), 4);
        assert_eq!(step(1, 5, 3, Dir::Down), 4);
        assert_eq!(step(2, 5, 3, Dir::Down), 4); // onto a short last row
        assert_eq!(step(4, 5, 3, Dir::Down), 4);
        assert_eq!(step(4, 5, 3, Dir::Up), 1);
        assert_eq!(step(0, 0, 3, Dir::Down), 0);
    }

    #[test]
    fn next_waiting_wraps_and_skips_working() {
        use State::*;
        let s = [Working, NeedsYou, Working, YourTurn, Starting];
        assert_eq!(next_waiting(&s, 0, true), 1);
        assert_eq!(next_waiting(&s, 1, true), 3);
        assert_eq!(next_waiting(&s, 3, true), 1);
        assert_eq!(next_waiting(&s, 1, false), 3);
        assert_eq!(next_waiting(&[Working, Working], 1, true), 1);
        assert_eq!(next_waiting(&[], 0, true), 0);
    }
}
