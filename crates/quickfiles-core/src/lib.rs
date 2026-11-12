//! quickfiles-core: everything behind the UI. No egui in here.
//!
//! Start with [`Core::start`]; read state from [`Core::nav`], [`Core::view`], [`Core::status`],
//! [`Core::data`], [`Core::drives`]; change it with the navigation and file methods.

mod compose;
mod core;
pub mod download;
pub mod dropdata;
pub mod engine;
mod fmt;
pub mod global_hotkey;
mod http;
pub mod images;
pub mod jobs;
mod model;
mod nav;
pub mod ole_drag;
pub mod ole_drop;
mod paging;
pub mod pathinput;
pub mod single_instance;
pub mod store;
pub mod sys;
pub mod tray;
mod view;
mod watch;

pub use crate::core::{Config, Core, CoreError};
pub use crate::engine::{EngineStatus, Wake};
pub use crate::fmt::{is_text_like, wants_thumbnail};
pub use crate::images::{
    Debounce, ImageKey, ImageStats, Images, JobKind, KindStats, Pixels, Prio, QuickKey, ScrollGate,
    preview_px,
};
pub use crate::jobs::{Conflict, JobInfo, JobState};
pub use crate::model::*;
pub use crate::nav::Nav;
pub use crate::store::Snapshot;
pub use crate::view::{EmptyReason, EntryRef, View, ViewState};
