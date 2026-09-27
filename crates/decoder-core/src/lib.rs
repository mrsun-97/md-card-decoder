//! Card Decoder engine core (no WASM dependencies).
//!
//! Layers: [`data`] (card table) → [`rules`] (judging) → [`kernel`] (partitions) → search and
//! simulation (added in later milestones; see PLAN.md §4).

pub mod actions;
pub mod belief;
pub mod cont;
pub mod ctx;
pub mod data;
pub mod kernel;
pub mod metrics;
pub mod policy;
pub mod recommend;
pub mod rng;
pub mod rules;
pub mod search;
pub mod sim;
pub mod synth;
pub mod value;

pub use belief::{Belief, Event};
pub use ctx::Ctx;
pub use data::{CardTable, Field, Gid, Pool};
pub use rules::{Fb, RulePolicy, Rules};
pub use value::Val;
