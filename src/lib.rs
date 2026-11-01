//! Core building blocks for the NyaScrapingCore MCU debugger frontend.
//!
//! The modules currently define project boundaries only; debugger integration
//! and command handling have not been implemented yet.

pub mod backend;
pub mod cli;
pub mod manager;
pub mod session;
