//! OmaFinances backend library. The binary (`main.rs`) wires these together; tests use them directly.

pub mod api;
pub mod cache;
pub mod config;
pub mod db;
pub mod http;
pub mod mail;
pub mod insight;
pub mod query;
pub mod report;
pub mod search;
pub mod smart;
pub mod secrets;
