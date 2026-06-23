//! RustVid 桌面客户端库入口
//!
//! 提供 `tauri::Builder` 启动所需的全部状态、命令、配置。
//! 入口在 `main.rs`(同 crate 的 binary target)。

pub mod app;
pub mod commands;
pub mod config;
pub mod domain;
pub mod persistence;
pub mod services;
