//! Cluster blueprints: roles/instances compiled to per-host deployments.
pub mod binding;
pub mod compile;
pub mod model;
#[allow(dead_code)] // wired into stack blueprint deployments by P2b Task 5
pub mod swarm;
pub mod template;
