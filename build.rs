//! 构建脚本:下载适配本平台的 FFmpeg 静态构建,让 RustVid 不依赖系统是否安装 FFmpeg。
//!
//! 来源:`eugeneware/ffmpeg-static` GitHub releases(单一跨平台源,直接是裸二进制,无需解压)。
//! 跨平台覆盖:
//! - darwin arm64 / x64
//! - linux arm64 / arm / x64 / ia32
//! - win32-x64
//!
//! 流程:
//! 1. 读 `TARGET` 映射到 asset 名
//! 2. 调 GitHub Releases API 拿 asset 的 `digest`(SHA256) — 缓存到 OUT_DIR 避免重复 API 调用
//! 3. 下载 ffmpeg
//! 4. 算本地 SHA256 与预期对比,不匹配视为下载失败(写 0 字节 dummy 回退系统 PATH)
//! 5. 设置 `cargo:rustc-env=BUNDLED_FFMPEG_PATH=<绝对路径>`,让 `include_bytes!` 能定位
//!
//! 跳过机制:`SKIP_BUNDLED_FFMPEG=1` 不下载,生成 0 字节 dummy,运行时回退到系统 PATH 上的 ffmpeg。

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const FFMPG_STATIC_VERSION: &str = "b6.1.1";
const FFMPG_STATIC_BASE: &str =
    "https://github.com/eugeneware/ffmpeg-static/releases/download";
const GITHUB_API_LATEST: &str =
    "https://api.github.com/repos/eugeneware/ffmpeg-static/releases/latest";

fn main() {
    // Tauri 2 构建(处理 tauri.conf.json、图标、capabilities)
    tauri_build::build();

    println!("cargo:rerun-if-env-changed=SKIP_BUNDLED_FFMPEG");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    let target = env::var("TARGET").expect("TARGET 未设置");

    if env::var_os("SKIP_BUNDLED_FFMPEG").is_some() {
        write_dummy(&out_dir);
        println!("cargo:warning=SKIP_BUNDLED_FFMPEG=1,跳过 bundled ffmpeg 下载,运行时回退到系统 PATH");
        return;
    }

    let Some(asset) = target_to_asset(&target) else {
        println!(
            "cargo:warning=目标 {target} 未在 build.rs 映射表中,运行时回退到系统 ffmpeg"
        );
        write_dummy(&out_dir);
        return;
    };

    let final_path = out_dir.join("ffmpeg");
    let sha_cache = out_dir.join("expected.sha256");

    // 让 cargo 在 ffmpeg 文件被改时重跑 build script(默认 cargo 不感知 ffmpeg 改动)
    println!("cargo:rerun-if-changed={}", final_path.display());

    // 即使文件已存在,也要校验 SHA256 —— 否则上次下载被截断/损坏的文件会被默默复用
    if final_path.exists() {
        if let (Ok(expected), Ok(actual)) = (
            load_or_fetch_expected_sha(asset, &sha_cache),
            compute_sha256(&final_path),
        ) {
            if actual.eq_ignore_ascii_case(&expected) {
                // 已存在且 SHA256 对,跳过下载
                emit_env(&final_path);
                return;
            }
            println!(
                "cargo:warning=已存在 ffmpeg 但 SHA256 不对(actual={actual} != expected={expected}),重新下载"
            );
            let _ = fs::remove_file(&final_path);
        } else {
            // 拿不到 SHA(API 失败 + 没 cache),保守起见重新下载
            println!("cargo:warning=无法校验已存在 ffmpeg 的 SHA256(可能 API 失败),重新下载保险");
            let _ = fs::remove_file(&final_path);
        }
    }

    let download_url = format!(
        "{FFMPG_STATIC_BASE}/{FFMPG_STATIC_VERSION}/ffmpeg-{asset}"
    );
    println!("cargo:warning=下载 bundled ffmpeg (asset: {asset}) ...");

    // 1. 从 GitHub Releases API 拿 asset 的预期 SHA256(digest 字段)
    // 缓存到 OUT_DIR/.sha256 避免反复调 API(60 req/h rate limit)
    let sha_cache = out_dir.join("expected.sha256");
    let expected_sha = match load_or_fetch_expected_sha(asset, &sha_cache) {
        Ok(s) => Some(s),
        Err(e) => {
            println!(
                "cargo:warning=GitHub API 拿 SHA256 失败:{e}。跳过哈希校验(下载可能不完整,转码时 verify_binary 会兜底)"
            );
            None
        }
    };

    // 2. 下载 ffmpeg
    if let Err(e) = download_to(&download_url, &final_path) {
        println!("cargo:warning=ffmpeg 下载失败:{e}。运行时回退到系统 PATH。");
        write_dummy(&out_dir);
        return;
    }

    // 3. 算 SHA256 与预期对比
    if let Some(expected) = expected_sha {
        let actual = match compute_sha256(&final_path) {
            Ok(s) => s,
            Err(e) => {
                println!(
                    "cargo:warning=算 SHA256 失败:{e}。删除下载文件,回退系统 PATH。"
                );
                let _ = fs::remove_file(&final_path);
                write_dummy(&out_dir);
                return;
            }
        };
        if !actual.eq_ignore_ascii_case(&expected) {
            println!(
                "cargo:warning=SHA256 校验失败:期望 {expected},实际 {actual}。删除下载文件,回退系统 PATH。"
            );
            let _ = fs::remove_file(&final_path);
            write_dummy(&out_dir);
            return;
        }
        println!("cargo:warning=SHA256 校验通过 ✓");
    }

    println!(
        "cargo:warning=bundled ffmpeg 已准备(asset:{asset}, 大小:{} 字节)",
        fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0)
    );
    emit_env(&final_path);
}

/// 把 Rust target triple 映射到 ffmpeg-static asset 名。
/// 返回 `None` 表示该 target 未支持(运行时回退到系统 ffmpeg)。
fn target_to_asset(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-apple-darwin" | "aarch64-apple-ios" => Some("darwin-arm64"),
        "x86_64-apple-darwin" => Some("darwin-x64"),
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => Some("linux-arm64"),
        "arm-unknown-linux-gnueabihf" | "arm-unknown-linux-musleabihf" => Some("linux-arm"),
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => Some("linux-x64"),
        "i686-unknown-linux-gnu" | "i686-unknown-linux-musl" => Some("linux-ia32"),
        "x86_64-pc-windows-msvc" => Some("win32-x64.exe"),
        _ => None,
    }
}

fn emit_env(path: &Path) {
    println!("cargo:rustc-env=BUNDLED_FFMPEG_PATH={}", path.display());
}

fn write_dummy(out_dir: &Path) {
    let path = out_dir.join("ffmpeg");
    fs::write(&path, b"").expect("写 dummy ffmpeg 文件失败");
    emit_env(&path);
}

fn download_to(url: &str, dest: &Path) -> io::Result<()> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| io::Error::other(format!("HTTP 请求失败:{e}")))?;
    let mut reader = response.into_reader();
    let mut file = fs::File::create(dest)?;
    io::copy(&mut reader, &mut file)?;
    file.flush()?;
    Ok(())
}

/// 拿 asset 的预期 SHA256:先看 OUT_DIR 缓存,没有再调 GitHub API
fn load_or_fetch_expected_sha(asset: &str, cache: &Path) -> io::Result<String> {
    if let Ok(c) = fs::read_to_string(cache) {
        // 缓存格式:`<asset_name>=<sha256>`
        if let Some(line) = c.lines().find(|l| l.starts_with(asset))
            && let Some((_, sha)) = line.split_once('=')
        {
            return Ok(sha.to_string());
        }
    }

    // 调 GitHub API 拿最新 release 的 asset list
    let body = fetch_release_metadata()?;
    let new_sha = parse_asset_sha256(&body, &format!("ffmpeg-{asset}"))
        .ok_or_else(|| io::Error::other(format!("release 资产列表中找不到 ffmpeg-{asset}")))?;

    // 追加到缓存(覆盖)
    let mut existing = fs::read_to_string(cache).unwrap_or_default();
    // 去掉同一 asset 的旧行
    existing = existing
        .lines()
        .filter(|l| !l.starts_with(asset))
        .collect::<Vec<_>>()
        .join("\n");
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&format!("{asset}={new_sha}\n"));
    fs::write(cache, existing)?;

    Ok(new_sha)
}

fn fetch_release_metadata() -> io::Result<String> {
    let response = ureq::get(GITHUB_API_LATEST)
        .set("User-Agent", "rustvid-build-script")
        .call()
        .map_err(|e| io::Error::other(format!("GitHub API 请求失败:{e}")))?;
    let mut body = String::new();
    response
        .into_reader()
        .read_to_string(&mut body)?;
    Ok(body)
}

/// 从 GitHub Releases API JSON 找 asset 的 SHA256(`digest` 字段格式 `sha256:<hex>`)
fn parse_asset_sha256(body: &str, asset_name: &str) -> Option<String> {
    // 极简 JSON 解析(避免引 serde_json):逐个资产对象扫描
    // 资产对象大概结构:
    //   {"name":"ffmpeg-darwin-arm64","digest":"sha256:abc...","size":45568216,...}
    // 不能简单 `find("\"name\":\"<name>\"")`,因为:
    //   - `ffmpeg-darwin-arm64` 是 `ffmpeg-darwin-arm64.gz` 的子串
    //   - 但两者有不同的引号后缀(`"` vs `."`)
    // 所以用 `:` 边界扫描,确保完整匹配 name
    let mut search_from = 0;
    while let Some(name_pos) = body[search_from..].find("\"name\":\"") {
        let abs_name_pos = search_from + name_pos + "\"name\":\"".len();
        // 找 name 结束(下一个 `"`)
        let name_end = body[abs_name_pos..].find('"')?;
        let name = &body[abs_name_pos..abs_name_pos + name_end];
        if name == asset_name {
            // 找 "digest":"<hash>"
            let digest_key = "\"digest\":\"";
            let after = abs_name_pos + name_end;
            let digest_pos_rel = body[after..].find(digest_key)?;
            let digest_start = after + digest_pos_rel + digest_key.len();
            let digest_end = body[digest_start..].find('"')?;
            let digest = &body[digest_start..digest_start + digest_end];
            return digest.strip_prefix("sha256:").map(|s| s.to_string());
        }
        search_from = abs_name_pos + name_end + 1;
    }
    None
}

fn compute_sha256(path: &Path) -> io::Result<String> {
    // 不引外部依赖,用系统 shasum(perl 兼容)或 sha256sum
    let output = Command::new("shasum")
        .arg("-a")
        .arg("256")
        .arg(path)
        .output();
    let output = match output {
        Ok(o) => o,
        Err(_) => Command::new("sha256sum")
            .arg(path)
            .output()
            .map_err(|e| io::Error::other(format!("shasum/sha256sum 都不可用:{e}")))?,
    };
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "sha256 命令失败 exit={:?}",
            output.status.code()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // 格式:`<hash>  <file>` (shasum / sha256sum 都是这样)
    stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .map(|s| s.to_string())
        .ok_or_else(|| io::Error::other("sha256 输出格式异常"))
}
