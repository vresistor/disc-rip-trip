//! Disc Rip Trip's window, in egui: the list of recipes, each drawn live
//! on a canvas from its run's snapshot, and the sidebar for what a step
//! needs from you. See DESIGN-UX.md and docs/adr/0001.

pub mod app;
pub mod canvas;
pub mod card;
pub mod chrome;
pub mod links;
pub mod model;
pub mod motion;
pub mod pace;
pub mod popover;
pub mod store;
pub mod theme;
pub mod tray;
pub mod widgets;

pub use app::{App, Appearance, Tray};
pub use model::{Deps, Model};
pub use store::Store;

/// The window, for eframe.
pub struct Window(pub App);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }

    /// Clear to nothing: the window draws its own rounded frame.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}
