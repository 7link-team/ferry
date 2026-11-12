//! Cluster blueprints: roles/instances compiled to per-host deployments.
pub mod binding;
pub mod compile;
// Consumed by the failover reconciler (P3 Task 6); drop the allow once it is wired in.
#[allow(dead_code)]
pub mod failover;
pub mod model;
pub mod placement;
pub mod spec;
pub mod swarm;
pub mod template;
