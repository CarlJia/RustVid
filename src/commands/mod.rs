//! Tauri command 集合
//!
//! 每个子模块对应一类功能,函数前加 `#[tauri::command]` 即可被前端 `invoke()` 调用。
//! 错误统一转 `String` 返回前端(tauri 限制 command 返回值必须可序列化)。

pub mod artifacts;
pub mod jobs;
pub mod presets;
pub mod uploads;
pub mod usage;
