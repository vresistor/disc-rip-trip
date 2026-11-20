//! The engine: recipes of steps, and running them.
//!
//! Words follow `GLOSSARY.md`. A [`Recipe`] is a tree of [`Node`]s; each step
//! node names a [`Step`] from the [`Registry`]. [`check`] types the tree;
//! [`Engine::start`] runs it, publishing a [`Snapshot`] as it changes and
//! taking [`Command`]s in. Nothing here knows about any tool or any window.

mod check;
pub mod context;
pub mod item;
pub mod lanes;
pub mod recipe;
pub mod run;
pub mod setting;
pub mod snapshot;
pub mod step;

pub use check::{check, first_misfit, fits, fits_flow, flow_after, flows_in, kind_after, offers, placements, CheckError, Flow, Offer};
pub use context::{Candidate, Context, Fact, Subject};
pub use item::{Item, ItemKind};
pub use lanes::LaneKind;
pub use recipe::{Node, Recipe, Spot};
pub use run::{cache_dir, Engine, Run};
pub use setting::{Fitted, ParamKind, ParamSpec, Scope, Setting, Settings, HELP_MAX};
pub use snapshot::{Answer, Choice, Command, Decision, ItemChoice, ItemState, ItemView, Made, Question, QuestionShape, Snapshot, StepState, StepView};
pub use step::{raw_here, within, Cx, DriveRead, Group, Inlet, Outlet, Registry, Step, StepInfo};
