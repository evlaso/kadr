//! kadr's screens as plain state, without a UI framework: what a screen
//! holds, what its commands do, undo. The GPUI screens (the app) only draw
//! it and turn input into these calls, so the behaviour is tested without a
//! window and another framework could draw the same thing.

pub mod editor;
pub mod history;
