//! The dashboard TUI that runs in tmux window 0.

pub mod app;
pub mod complete;
mod flair;
pub mod grid;
pub mod help;
pub mod input;
pub mod render;
pub mod run;
pub mod search;

#[cfg(test)]
mod tests;
