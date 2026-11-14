//! Widget ids. egui 0.37 deprecated `Id::new` in favour of `Id::unique` / `IdSalt::new`; every id
//! in the app is made here, so moving to the new API is a change to this one function.

use std::hash::Hash;

use eframe::egui::Id;

/// An `Id` made from any hashable name: the same name always gives the same id.
#[allow(deprecated)]
pub fn id(source: impl Hash + std::fmt::Debug) -> Id {
    Id::new(source)
}
