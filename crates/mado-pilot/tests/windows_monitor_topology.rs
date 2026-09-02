#![cfg_attr(not(windows), allow(missing_docs))]
#![cfg(windows)]
//! Runs the benchmark-only Windows monitor-topology regressions.

#[allow(dead_code)]
#[path = "../benches/support/windows_monitor_topology.rs"]
mod windows_monitor_topology;
