//! Core building blocks for the NyaScrapingCore MCU debugger frontend.
//!
//! Provides debugger version management, common backend commands, and GDB
//! session lifecycle management.

pub mod backend;
pub mod cli;
pub mod manager;
pub mod session;
