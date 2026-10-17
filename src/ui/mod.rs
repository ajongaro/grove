//! The dashboard TUI that runs in tmux window 0.

pub mod app;
pub mod complete;
pub mod grid;
pub mod help;
pub mod input;
pub mod render;
pub mod run;

#[cfg(test)]
mod tests;
