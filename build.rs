//! 构建脚本:下载适配本平台的 FFmpeg 静态构建,让 RustVid 不依赖系统是否安装 FFmpeg。
//!
//! 来源(按平台分流):
//! - macOS: eugeneware/ffmpeg-static(BtbN 不提供 macOS build)
//! - Linux x64 / arm64、Windows x64: BtbN/FFmpeg-Builds
//! - 其他架构(linux arm / i686 等,BtbN 不提供):eugeneware/ffmpeg-static 兜底
//!
//! eugeneware:发布的就是裸二进制文件,直接下到 OUT_DIR/ffmpeg
//! BtbN:发布的是 .tar.xz / .zip 压缩包,内部 bin/ffmpeg[.exe],需要解压
//!
//! 流程:
//! 1. 读 `TARGET` 映射到 ffmpeg 来源(enum: Eugeneware / Btbn)
//! 2. 拿预期 SHA256(eugeneware 调 GitHub API digest,BtbN 下 checksums.sha256)
//! 3. 下载;BtbN 还要解压提取 bin/ffmpeg[.exe]
//! 4. 校验 SHA256(eugeneware 校验 ffmpeg 二进制,BtbN 校验压缩包)
//! 5. 设置 `cargo:rustc-env=BUNDLED_FFMPEG_PATH=<绝对路径>`
//!
//! 跳过机制:`SKIP_BUNDLED_FFMPEG=1` 不下载,生成 0 字节 dummy,运行时回退到系统 PATH。

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

// === eugeneware/ffmpeg-static (macOS + Linux 兜底) ===
const FFMPG_STATIC_VERSION: &str = "b6.1.1";
const FFMPG_STATIC_BASE: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download";
const FFMPG_STATIC_API: &str =
    "https://api.github.com/repos/eugeneware/ffmpeg-static/releases/latest";

// === BtbN/FFmpeg-Builds (Linux x64/arm64 + Windows x64) ===
const BTBN_VERSION: &str = "latest";
const BTBN_BASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";
const BTBN_CHECKSUMS_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/checksums.sha256";

#[derive(Debug, Clone, Copy)]
enum ArchiveFormat {
    TarXz,
    Zip,
}

#[derive(Debug, Clone, Copy)]
enum FfmpegSource {
    /// eugeneware/ffmpeg-static — 裸二进制(无后缀,macOS/Linux 直接是 ffmpeg)
    Eugeneware { asset: &'static str },
    /// BtbN/FFmpeg-Builds — .tar.xz / .zip 压缩包,内含 bin/ffmpeg
    Btbn {
        asset_stem: &'static str,
        archive: ArchiveFormat,
    },
}

impl FfmpegSource {
    fn label(&self) -> String {
        match self {
            FfmpegSource::Eugeneware { asset } => format!("eugeneware:{asset}"),
            FfmpegSource::Btbn { asset_stem, .. } => format!("BtbN:{asset_stem}"),
        }
    }
}

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
        println!(
            "cargo:warning=SKIP_BUNDLED_FFMPEG=1,跳过 bundled ffmpeg 下载,运行时回退到系统 PATH"
        );
        return;
    }

    let Some(source) = target_to_ffmpeg_source(&target) else {
        println!("cargo:warning=目标 {target} 未在 build.rs 映射表中,运行时回退到系统 ffmpeg");
        write_dummy(&out_dir);
        return;
    };

    let final_path = out_dir.join("ffmpeg");
    println!("cargo:rerun-if-changed={}", final_path.display());

    // 已存在且 hash 仍有效 → 复用
    if final_path.exists() && source.verify_existing(&final_path, &out_dir) {
        emit_env(&final_path);
        return;
    }
    let _ = fs::remove_file(&final_path);

    if let Err(e) = fetch_and_prepare(&out_dir, source, &final_path) {
        println!("cargo:warning=ffmpeg 准备失败:{e}。运行时回退到系统 PATH。");
        let _ = fs::remove_file(&final_path);
        write_dummy(&out_dir);
        return;
    }

    println!(
        "cargo:warning=bundled ffmpeg 已准备(source:{}, 大小:{} 字节)",
        source.label(),
        fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0)
    );
    emit_env(&final_path);
}

/// 把 Rust target triple 映射到 ffmpeg 来源。
/// 返回 `None` 表示该 target 未支持(运行时回退到系统 ffmpeg)。
fn target_to_ffmpeg_source(target: &str) -> Option<FfmpegSource> {
    match target {
        // === macOS:BtbN 不提供 macOS build,继续用 eugeneware ===
        "aarch64-apple-darwin" | "aarch64-apple-ios" => Some(FfmpegSource::Eugeneware {
            asset: "darwin-arm64",
        }),
        "x86_64-apple-darwin" => Some(FfmpegSource::Eugeneware {
            asset: "darwin-x64",
        }),

        // === Windows:用 BtbN ===
        "x86_64-pc-windows-msvc" => Some(FfmpegSource::Btbn {
            asset_stem: "win64-gpl",
            archive: ArchiveFormat::Zip,
        }),

        // === Linux x86_64:用 BtbN ===
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => Some(FfmpegSource::Btbn {
            asset_stem: "linux64-gpl",
            archive: ArchiveFormat::TarXz,
        }),

        // === Linux arm64:用 BtbN ===
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => Some(FfmpegSource::Btbn {
            asset_stem: "linuxarm64-gpl",
            archive: ArchiveFormat::TarXz,
        }),

        // === 兜底:BtbN 不提供这些架构,继续用 eugeneware ===
        "arm-unknown-linux-gnueabihf" | "arm-unknown-linux-musleabihf" => {
            Some(FfmpegSource::Eugeneware { asset: "linux-arm" })
        }
        "i686-unknown-linux-gnu" | "i686-unknown-linux-musl" => Some(FfmpegSource::Eugeneware {
            asset: "linux-ia32",
        }),

        _ => None,
    }
}

impl FfmpegSource {
    /// 检查已存在的 final_path 是否仍是这个 source 的有效产物。
    /// 返回 true 表示可跳过下载。
    fn verify_existing(&self, final_path: &Path, out_dir: &Path) -> bool {
        match self {
            FfmpegSource::Eugeneware { asset } => {
                // eugeneware:缓存的 hash 就是 ffmpeg 二进制本身的 SHA256
                let cache = out_dir.join("expected.sha256");
                let Ok(expected) = load_or_fetch_eugeneware_sha(asset, &cache) else {
                    return false;
                };
                let Ok(actual) = compute_sha256(final_path) else {
                    return false;
                };
                actual.eq_ignore_ascii_case(&expected)
            }
            FfmpegSource::Btbn { .. } => {
                // BtbN 的 checksums.sha256 给的是压缩包 hash,不是 ffmpeg 二进制 hash,
                // 没法用 final_path 反查对应的压缩包版本,所以总是重新下载(压缩包 ~80MB,可接受)
                false
            }
        }
    }
}

fn fetch_and_prepare(out_dir: &Path, source: FfmpegSource, final_path: &Path) -> io::Result<()> {
    match source {
        FfmpegSource::Eugeneware { asset } => fetch_eugeneware(out_dir, asset, final_path),
        FfmpegSource::Btbn {
            asset_stem,
            archive,
        } => fetch_btbn(out_dir, asset_stem, archive, final_path),
    }
}

// === eugeneware 流程 ===

fn fetch_eugeneware(out_dir: &Path, asset: &str, final_path: &Path) -> io::Result<()> {
    let download_url = format!("{FFMPG_STATIC_BASE}/{FFMPG_STATIC_VERSION}/ffmpeg-{asset}");
    println!("cargo:warning=下载 bundled ffmpeg (eugeneware:{asset}) ...");

    // 1. 拿预期 SHA(走 GitHub API,失败则跳过校验)
    let sha_cache = out_dir.join("expected.sha256");
    let expected_sha = match load_or_fetch_eugeneware_sha(asset, &sha_cache) {
        Ok(s) => Some(s),
        Err(e) => {
            println!(
                "cargo:warning=GitHub API 拿 SHA256 失败:{e}。跳过哈希校验(下载可能不完整,转码时 verify_binary 会兜底)"
            );
            None
        }
    };

    // 2. 下载
    download_to(&download_url, final_path)?;

    // 3. 校验
    if let Some(expected) = expected_sha {
        let actual = compute_sha256(final_path)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            let _ = fs::remove_file(final_path);
            return Err(io::Error::other(format!(
                "SHA256 校验失败:期望 {expected},实际 {actual}"
            )));
        }
        println!("cargo:warning=SHA256 校验通过 ✓");
    }
    Ok(())
}

// === BtbN 流程 ===

fn fetch_btbn(
    out_dir: &Path,
    asset_stem: &str,
    archive: ArchiveFormat,
    final_path: &Path,
) -> io::Result<()> {
    let archive_name = match archive {
        ArchiveFormat::TarXz => format!("ffmpeg-master-{BTBN_VERSION}-{asset_stem}.tar.xz"),
        ArchiveFormat::Zip => format!("ffmpeg-master-{BTBN_VERSION}-{asset_stem}.zip"),
    };
    let download_url = format!("{BTBN_BASE}/{archive_name}");
    println!("cargo:warning=下载 bundled ffmpeg (BtbN:{archive_name}) ...");

    // 1. 从 BtbN checksums.sha256 拿预期 SHA(压缩包本身)
    let expected_sha = fetch_btbn_asset_sha(&archive_name)?;

    // 2. 下载到临时路径
    let tmp_archive = out_dir.join("ffmpeg-archive.tmp");
    download_to(&download_url, &tmp_archive)?;

    // 3. 校验压缩包 SHA
    let actual = compute_sha256(&tmp_archive)?;
    if !actual.eq_ignore_ascii_case(&expected_sha) {
        let _ = fs::remove_file(&tmp_archive);
        return Err(io::Error::other(format!(
            "BtbN 压缩包 SHA256 校验失败:期望 {expected_sha},实际 {actual}"
        )));
    }
    println!("cargo:warning=SHA256 校验通过 ✓");

    // 4. 解压,提取 bin/ffmpeg[.exe] -> final_path
    if let Err(e) = extract_btbn_ffmpeg(&tmp_archive, archive, final_path) {
        let _ = fs::remove_file(&tmp_archive);
        return Err(e);
    }
    let _ = fs::remove_file(&tmp_archive);
    Ok(())
}

fn fetch_btbn_asset_sha(archive_name: &str) -> io::Result<String> {
    let mut body = String::new();
    let response = ureq::get(BTBN_CHECKSUMS_URL)
        .set("User-Agent", "rustvid-build-script")
        .call()
        .map_err(|e| io::Error::other(format!("BtbN checksums 下载失败:{e}")))?;
    response
        .into_reader()
        .read_to_string(&mut body)
        .map_err(|e| io::Error::other(format!("读 BtbN checksums 失败:{e}")))?;

    // 格式:`<hash>  <filename>`(`read_to_string` 不会去掉末尾换行,逐行扫)
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(hash_raw), Some(name)) = (parts.next(), parts.next())
            && name == archive_name
        {
            return parse_hash_token(hash_raw);
        }
    }
    Err(io::Error::other(format!(
        "BtbN checksums.sha256 中找不到 {archive_name}"
    )))
}

fn extract_btbn_ffmpeg(archive: &Path, format: ArchiveFormat, dest: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    match format {
        ArchiveFormat::TarXz => {
            let xz = xz2::read::XzDecoder::new(file);
            let mut tar = tar::Archive::new(xz);
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry.path()?;
                // BtbN 目录结构:`ffmpeg-master-latest-linux64-gpl/bin/ffmpeg`
                let is_ffmpeg_bin = path.components().any(|c| c.as_os_str() == "bin")
                    && path
                        .file_name()
                        .is_some_and(|n| n == "ffmpeg" || n == "ffmpeg.exe");
                if is_ffmpeg_bin {
                    let mut out = fs::File::create(dest)?;
                    io::copy(&mut entry, &mut out)?;
                    out.flush()?;
                    return Ok(());
                }
            }
            Err(io::Error::other("BtbN 压缩包中找不到 bin/ffmpeg[.exe]"))
        }
        ArchiveFormat::Zip => {
            let mut zip = zip::ZipArchive::new(file)?;
            let mut target_idx = None;
            for i in 0..zip.len() {
                let entry = zip.by_index(i)?;
                let name = entry.name();
                if name.contains("/bin/")
                    && (name.ends_with("/ffmpeg") || name.ends_with("/ffmpeg.exe"))
                {
                    target_idx = Some(i);
                    break;
                }
            }
            let idx =
                target_idx.ok_or_else(|| io::Error::other("BtbN zip 中找不到 bin/ffmpeg[.exe]"))?;
            let mut entry = zip.by_index(idx)?;
            let mut out = fs::File::create(dest)?;
            io::copy(&mut entry, &mut out)?;
            out.flush()?;
            Ok(())
        }
    }
}

// === 通用 helper ===

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

/// 拿 eugeneware asset 的预期 SHA256:先看 OUT_DIR 缓存,没有再调 GitHub API
fn load_or_fetch_eugeneware_sha(asset: &str, cache: &Path) -> io::Result<String> {
    if let Ok(c) = fs::read_to_string(cache) {
        // 缓存格式:`<asset_name>=<sha256>`
        if let Some(line) = c.lines().find(|l| l.starts_with(asset))
            && let Some((_, sha)) = line.split_once('=')
        {
            return Ok(sha.to_string());
        }
    }

    // 调 GitHub API 拿最新 release 的 asset list
    let body = fetch_eugeneware_release_metadata()?;
    let new_sha = parse_asset_sha256(&body, &format!("ffmpeg-{asset}"))
        .ok_or_else(|| io::Error::other(format!("release 资产列表中找不到 ffmpeg-{asset}")))?;

    // 追加到缓存(覆盖同 asset 的旧行)
    let mut existing = fs::read_to_string(cache).unwrap_or_default();
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

fn fetch_eugeneware_release_metadata() -> io::Result<String> {
    let response = ureq::get(FFMPG_STATIC_API)
        .set("User-Agent", "rustvid-build-script")
        .call()
        .map_err(|e| io::Error::other(format!("GitHub API 请求失败:{e}")))?;
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body)?;
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
    // Windows runner 默认有 sha256sum(Git for Windows 自带),但 shasum 不一定有,
    // 用 shasum 优先是因为 macOS 默认有,Linux 通常两者都有
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
            "sha256 命令失败 exit={:?},stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw_token = stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| {
            io::Error::other(format!(
                "sha256 输出格式异常:stdout='{}'",
                stdout.chars().take(200).collect::<String>()
            ))
        })?;
    parse_hash_token(raw_token)
}

/// 从 shasum/sha256sum 输出或 BtbN checksums 文件中解析 SHA256 hex。
/// 容忍以下异常前缀(coreutils binary mode 标记 `*`、Windows path 残留 `\`、CRLF):
/// - `<hash>  *<file>`  → `<hash>`(剥 `*` 前缀)
/// - `\b2522...  <file>`  → `<hash>`(剥 `\` 前缀,Windows 上 Git bash shasum 偶尔出现)
/// 严格校验:必须是 64 字符 hex,否则返回错(防止脏数据当 hash 用)
fn parse_hash_token(raw: &str) -> io::Result<String> {
    let trimmed = raw.trim().trim_start_matches(['*', '\\']);
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_ascii_lowercase())
    } else {
        Err(io::Error::other(format!(
            "不是 64 字符 hex SHA256:raw='{}'(len={})",
            &raw[..raw.len().min(40)],
            raw.len()
        )))
    }
}
