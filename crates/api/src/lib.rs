//! OncallKit API server, background worker and domain logic.

// Query results are read into tuples where a named row struct would add nothing.
#![allow(clippy::type_complexity)]

pub mod audit;
pub mod auth;
pub mod calendar;
pub mod config;
pub mod crypto;
pub mod error;
pub mod integrations;
pub mod notifications;
pub mod observability;
pub mod organizations;
pub mod pagination;
pub mod rbac;
pub mod routes;
pub mod schedules;
pub mod seed;
pub mod state;
pub mod swaps;
pub mod teams;
pub mod worker;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
