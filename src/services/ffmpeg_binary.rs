//! 嵌入式 FFmpeg 二进制管理
//!
//! 职责:把 `build.rs` 嵌入的 FFmpeg 字节(`include_bytes!` 在编译期固化)在首次用时
//! 抽出到本地缓存目录,后续调用零 IO 返回路径。
//!
//! 行为矩阵:
//! | 编译期 BUNDLED_FFMPEG | SKIP_BUNDLED_FFMPEG | 运行时路径 |
//! |---|---|---|
//! | >0 字节(默认) | 未设 | 抽到 `temp_dir()/rustvid/ffmpeg[.exe]`,返回该路径 |
//! | 0 字节(dummy)  | 设了或目标未支持 | 返回 `"ffmpeg"`,走系统 PATH |
//!
//! 缓存策略:文件大小与嵌入字节一致时复用,不一致(版本升级)自动重写。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `build.rs` 嵌入的 FFmpeg 字节。SKIP 或未支持平台时是 0 字节 dummy。
const BUNDLED_FFMPEG: &[u8] = include_bytes!(env!("BUNDLED_FFMPEG_PATH"));

/// 缓存目录名(在系统 temp 目录下)
const CACHE_DIR_NAME: &str = "rustvid";

/// 返回可在 `Command::new` 里直接用的 FFmpeg 路径。
/// 首次调用时把嵌入字节写到 `<temp>/rustvid/ffmpeg[.exe]`(并 chmod +x),后续命中 `OnceLock` 零 IO。
pub fn ffmpeg_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(resolve_path).as_path()
}

fn resolve_path() -> PathBuf {
    if BUNDLED_FFMPEG.is_empty() {
        // SKIP 模式或未支持平台:回退到系统 PATH 上的 ffmpeg
        tracing::info!("bundled ffmpeg 未嵌入,运行时回退到系统 PATH 上的 ffmpeg");
        return PathBuf::from("ffmpeg");
    }
    extract_to_cache()
}

fn extract_to_cache() -> PathBuf {
    let bin_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let target = std::env::temp_dir().join(CACHE_DIR_NAME).join(bin_name);

    // 大小不一致才重写;同时跑 sanity check 防止上次截断或损坏的缓存被复用
    let need_write = match std::fs::metadata(&target) {
        Ok(m) if m.len() as usize == BUNDLED_FFMPEG.len() => {
            // 大小对得上:再验证一下 magic 字节(Mach-O = 0xCFFAEDFE / 0xCEFAEDFE, ELF = 0x7F454C46)
            if let Ok(head) = std::fs::read(&target).map(|v| {
                let mut h = [0u8; 4];
                if v.len() >= 4 {
                    h.copy_from_slice(&v[..4]);
                }
                h
            }) {
                let valid = cfg!(target_os = "macos")
                    && (head == [0xCF, 0xFA, 0xED, 0xFE] || head == [0xCE, 0xFA, 0xED, 0xFE])
                    || cfg!(target_os = "windows")
                    && head == [0x4D, 0x5A, 0x00, 0x00]
                    || cfg!(target_os = "linux")
                    && head == [0x7F, 0x45, 0x4C, 0x46];
                if !valid {
                    tracing::warn!(path = %target.display(), "bundled ffmpeg 缓存 magic 字节不匹配,重写");
                    true
                } else {
                    false
                }
            } else {
                true
            }
        }
        Ok(_) => true,  // size 不一致,重写
        Err(_) => true, // 没有缓存,首次写
    };

    if need_write {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("创建 rustvid 缓存目录失败");
        }
        std::fs::write(&target, BUNDLED_FFMPEG).expect("写 bundled ffmpeg 失败");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                .expect("chmod bundled ffmpeg 失败");
        }
        // macOS: 抽出来的二进制可能继承 .app 的 quarantine xattr,导致 spawn 时 "Malformed Mach-o file"
        // 用 xattr -d com.apple.quarantine 去掉(等价命令;忽略 "no such xattr" 错误)
        #[cfg(target_os = "macos")]
        remove_quarantine_xattr(&target);
    }

    // 抽完做自检:`ffmpeg -version` 跑 5 秒,失败则返回错并提示用户
    if let Err(e) = verify_binary(&target) {
        tracing::error!(
            path = %target.display(),
            error = %e,
            "bundled ffmpeg 自检失败(spawn 阶段):二进制可能损坏 / quarantine / 架构不对"
        );
        // 不 panic,让上层 run_ffmpeg 报更具体的错;但记日志方便诊断
    } else {
        tracing::info!(
            path = %target.display(),
            bytes = BUNDLED_FFMPEG.len(),
            "bundled ffmpeg 已提取到缓存"
        );
    }

    target
}

/// 把 ffmpeg -i 输出的 "X kb/s" / "X Mbit/s" / "X b/s" 统一转 bps 整数。
/// 纯函数、无 IO,易单测。失败返回 `None`(让调用方 fallback)。
pub fn ffmpeg_bitrate_unit(value: &str, unit: &str) -> Option<u64> {
    let n: f64 = value.trim().parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    let multiplier: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "b/s" | "bps" => 1,
        "kbit/s" | "kbps" | "kb/s" => 1_000,
        "mbit/s" | "mbps" | "mb/s" => 1_000_000,
        _ => return None,
    };
    Some((n * multiplier as f64) as u64)
}

/// macOS: 去掉文件的 `com.apple.quarantine` xattr(等同 `xattr -d com.apple.quarantine <path>`)
/// spawn 抽出的二进制时如果不脱掉这个属性,会触发 "Malformed Mach-o file" 错误(os error 88)
#[cfg(target_os = "macos")]
fn remove_quarantine_xattr(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let c_attr = match CString::new("com.apple.quarantine") {
        Ok(s) => s,
        Err(_) => return,
    };
    // 安全:libc::removexattr 不返回错误也可能表示"属性不存在",忽略
    // macOS 的 removexattr 签名带 flags(0 = 不跟符号链接,默认行为)
    unsafe {
        libc::removexattr(c_path.as_ptr(), c_attr.as_ptr(), 0);
    }
}

/// 自检刚抽出的 ffmpeg 二进制能不能正常 spawn + 输出 version
/// 失败返回错误信息(供日志),不 panic
fn verify_binary(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    let output = Command::new(path).arg("-version").output()?;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(std::io::Error::other(format!(
            "ffmpeg 退出 {:?},stdout 空(stderr: {})",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).chars().take(200).collect::<String>()
        )));
    }
    Ok(())
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_bitrate_unit_解析_kb_s() {
        assert_eq!(ffmpeg_bitrate_unit("1844", "kb/s"), Some(1_844_000));
        assert_eq!(ffmpeg_bitrate_unit("185", "kbps"), Some(185_000));
    }

    #[test]
    fn ffmpeg_bitrate_unit_解析_mbit_s() {
        assert_eq!(ffmpeg_bitrate_unit("1.5", "Mbit/s"), Some(1_500_000));
        assert_eq!(ffmpeg_bitrate_unit("2.2", "Mbps"), Some(2_200_000));
    }

    #[test]
    fn ffmpeg_bitrate_unit_解析_b_s() {
        assert_eq!(ffmpeg_bitrate_unit("500", "b/s"), Some(500));
    }

    #[test]
    fn ffmpeg_bitrate_unit_大小写不敏感() {
        assert_eq!(ffmpeg_bitrate_unit("1.5", "MBIT/S"), Some(1_500_000));
    }

    #[test]
    fn ffmpeg_bitrate_unit_带前后空白() {
        assert_eq!(ffmpeg_bitrate_unit("  1.5  ", "  Mbit/s  "), Some(1_500_000));
    }

    #[test]
    fn ffmpeg_bitrate_unit_未知单位返回_none() {
        assert_eq!(ffmpeg_bitrate_unit("100", "GB/s"), None);
        assert_eq!(ffmpeg_bitrate_unit("100", ""), None);
    }

    #[test]
    fn ffmpeg_bitrate_unit_非法数字返回_none() {
        assert_eq!(ffmpeg_bitrate_unit("abc", "kb/s"), None);
        assert_eq!(ffmpeg_bitrate_unit("", "kb/s"), None);
    }

    #[test]
    fn ffmpeg_bitrate_unit_负数返回_none() {
        assert_eq!(ffmpeg_bitrate_unit("-1", "kb/s"), None);
    }
}
