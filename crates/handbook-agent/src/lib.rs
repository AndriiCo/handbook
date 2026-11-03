//! Analysis agent: model clients, response validation, and the
//! watermarked draft→validate→store runner. Programs to the
//! [`contracts::model::ModelClient`] trait; never auto-accepts.

pub mod analyze;
pub mod changes;
pub mod model;
pub mod replay;
pub mod runner;
