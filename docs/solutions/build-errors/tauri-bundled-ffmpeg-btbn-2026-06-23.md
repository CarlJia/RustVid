---
title: Tauri 2 跨平台打包 ffmpeg 来源切到 BtbN/FFmpeg-Builds
date: 2026-06-23
category: build-errors
module: build.rs (Tauri 2 / FFmpeg bundling & CI release workflow)
problem_type: build_error
component: development_workflow
severity: high
symptoms:
  - "Windows CI: ffmpeg download 404 — eugeneware asset 名错配成 `win32-x64.exe`(实际是 `win32-x64`,无 .exe)"
  - "Linux CI: tauri build 链接 `libwebkit2gtk-4.1.so.0` 失败(ubuntu-22.04 runner 未装 webkit/gtk3 等系统依赖)"
  - "PowerShell SHA256SUMS 步骤 parser error: `} | ForEach-Object { $entries += $_ }` 在 `}` 闭合后接 `|` 不被 PowerShell 接受"
  - "macOS CI: `Get-FileHash` 报 `Access to the path ... is denied`,因为 `*.app` 实际是 bundle 目录,Get-FileHash 不能 hash 目录"
  - "actions/checkout@v4 / setup-node@v4 触发 Node.js 20 弃用警告(2025-09-19 弃用)"
root_cause: config_error
resolution_type: config_change
related_components:
  - .github/workflows/release.yml
  - .github/workflows/ci.yml
  - Cargo.toml
  - v0.1.0 git tag
tags:
  - tauri
  - ffmpeg
  - btbn
  - eugeneware
  - ci
  - github-actions
  - powershell
  - cross-platform
---

# Tauri 2 跨平台打包 ffmpeg 来源切到 BtbN/FFmpeg-Builds

## Problem

RustVid 的 `build.rs` 之前只从 `eugeneware/ffmpeg-static` 一个源拉 ffmpeg 二进制,这带来三个问题:

1. **Windows asset 名错配**:`eugeneware` 的 Windows asset 是 `ffmpeg-win32-x64`(无 `.exe` 后缀),但 build.rs 写成 `win32-x64.exe`,下到 GitHub 返回 404
2. **codec 与 license 不足**:BtbN/FFmpeg-Builds 的 gpl 版本内置 libfdk-aac / libx264 等,codec 覆盖和 GPL 兼容性比 eugeneware 强
3. **跨平台抽象缺失**:没有 enum 化的 source 抽象,加新平台要在散落的 `if windows { ... }` 分支里改

BtbN 不提供 macOS build,所以必须**双源**:macOS 继续 eugeneware,Windows / Linux 切 BtbN。

## Symptoms

- Windows CI:`cargo build` 阶段 `cargo:warning=ffmpeg 下载失败:HTTP 请求失败:status code 404`,运行时回退到系统 PATH,但 GitHub Actions ubuntu runner 上没装系统 ffmpeg,后续 `tauri build` 失败
- Linux CI:`tauri build` 报 `Package gdk-3.0 was not found`,因为 ubuntu-22.04 runner 默认没装 `libwebkit2gtk-4.1-dev` / `libgtk-3-dev` 等 Tauri 系统依赖
- PowerShell 步骤:`ParserError: An empty pipe element is not allowed`,line 20 指向 `} | ForEach-Object { $entries += $_ }`
- macOS CI:`Get-FileHash: ... Access to the path ... is denied`,因为 `*.app` 在 macOS 上是 bundle 目录
- Actions 日志:`Node.js 20 is deprecated. The following actions target Node.js 20 but are being forced to run on Node.js 24`

## What Didn't Work

- **只改 asset 名去掉 `.exe`**:能修 Windows,但 eugeneware 本身 codec/license 不够,且未来 macOS 切 BtbN 也不可能(BtbN 不提供 macOS),治标不治本
- **统一把 Linux 也切到 eugeneware 修好的 Windows 路径**:eugeneware 缺乏小众架构(Linux arm / i686 等)官方 build,且 codec 覆盖不足
- **尝试校验 BtbN 解压后 ffmpeg 的 SHA256**:BtbN `checksums.sha256` 只给压缩包 hash,没法用解压后产物反查,这条路走不通
- **试图用 eugeneware 那套 SHA 缓存跳过 BtbN 重复下载**:BtbN checksum 是压缩包 hash,与最终 `final_path`(已解压的 ffmpeg)对不上,无法做"已有产物 hash 是否匹配"的复用判断
- **PowerShell `foreach { ... } | ForEach-Object { $entries += $_ }`**:PowerShell parser 不接受语句块闭合后直接 `|` 接命令

## Solution

### 1. build.rs: 引入 `FfmpegSource` enum 抽象

```rust
#[derive(Debug, Clone, Copy)]
enum ArchiveFormat { TarXz, Zip }

#[derive(Debug, Clone, Copy)]
enum FfmpegSource {
    /// eugeneware/ffmpeg-static — 裸二进制(无后缀,macOS/Linux 直接是 ffmpeg)
    Eugeneware { asset: &'static str },
    /// BtbN/FFmpeg-Builds — .tar.xz / .zip 压缩包,内含 bin/ffmpeg
    Btbn { asset_stem: &'static str, archive: ArchiveFormat },
}
```

### 2. target → source 映射表

| target triple | Source | asset / asset_stem |
|---|---|---|
| `aarch64-apple-darwin` / `-ios` | Eugeneware | `darwin-arm64` |
| `x86_64-apple-darwin` | Eugeneware | `darwin-x64` |
| `x86_64-pc-windows-msvc` | **Btbn** | `win64-gpl` (.zip) |
| `x86_64-unknown-linux-gnu` / `-musl` | **Btbn** | `linux64-gpl` (.tar.xz) |
| `aarch64-unknown-linux-gnu` / `-musl` | **Btbn** | `linuxarm64-gpl` (.tar.xz) |
| `arm-*-linux-*` | Eugeneware 兜底 | `linux-arm` |
| `i686-*-linux-*` | Eugeneware 兜底 | `linux-ia32` |
| 其他 | `None` → 写 dummy,运行时回退系统 PATH | — |

BtbN 不提供 macOS build 是上游事实,代码注释里写明避免后人误解。

### 3. BtbN 下载 + 校验 + 解压

```rust
fn fetch_btbn(out_dir: &Path, asset_stem: &str, archive: ArchiveFormat, final_path: &Path) -> io::Result<()> {
    let archive_name = match archive {
        ArchiveFormat::TarXz => format!("ffmpeg-master-{BTBN_VERSION}-{asset_stem}.tar.xz"),
        ArchiveFormat::Zip   => format!("ffmpeg-master-{BTBN_VERSION}-{asset_stem}.zip"),
    };
    let download_url = format!("{BTBN_BASE}/{archive_name}");

    // 1. 从 BtbN checksums.sha256 拿预期 SHA
    let expected = fetch_btbn_asset_sha(&archive_name)?;
    // 2. 下载到 tmp,校验压缩包 SHA
    let tmp = out_dir.join("ffmpeg-archive.tmp");
    download_to(&download_url, &tmp)?;
    let actual = compute_sha256(&tmp)?;
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(io::Error::other("BtbN 压缩包 SHA256 校验失败"));
    }
    // 3. 解压,提取 bin/ffmpeg[.exe] -> final_path
    extract_btbn_ffmpeg(&tmp, archive, final_path)?;
    let _ = fs::remove_file(&tmp);
    Ok(())
}
```

解压时按 entries 顺序扫,**`bin/` 路径前缀 + 精确文件名 `ffmpeg` 或 `ffmpeg.exe`** 双匹配,避免误取 `bin/ffprobe` 或 `LICENSE`。

### 4. SHA256 校验粒度差异(设计取舍)

| Source | 校验对象 | 来源 |
|---|---|---|
| Eugeneware | ffmpeg **二进制** | GitHub Releases API `digest` 字段(`sha256:` 前缀) |
| BtbN | ffmpeg **压缩包** | BtbN `checksums.sha256` 中对应行 |

**二次构建优化**:
- Eugeneware:复用原 SHA 缓存(`OUT_DIR/expected.sha256` 存 `<asset>=<sha>`)
- BtbN:总是重新下载(BtbN checksum 与 final_path hash 不同源,无法做"产物 hash 反查")

BtbN 压缩包 ~80MB,设计取舍是**正确性 > 缓存命中率**。

### 5. CI 配套修复

```yaml
# .github/workflows/ci.yml: 加 Linux 系统依赖步骤
- name: 安装 Linux 系统依赖(Tauri 编译需要)
  if: runner.os == 'Linux'
  run: |
    sudo apt-get update
    sudo apt-get install -y \
      libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev \
      librsvg2-dev patchelf libssl-dev
```

```yaml
# .github/workflows/release.yml: actions 升 v5
- uses: actions/checkout@v5
- uses: actions/setup-node@v5
```

PowerShell SHA256SUMS 重写,把 `foreach { ... } | ForEach-Object { $entries += $_ }` 换成 `$entries = foreach { ... }` 表达式赋值:

```powershell
$entries = foreach ($pat in $patterns) {
  Get-ChildItem -Path $pat -ErrorAction SilentlyContinue -File | ForEach-Object {
    $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    "$h  $($_.Directory.Name)/$($_.Name)"
  }
}
$entries | Out-File -FilePath $sumFile -Encoding utf8
```

同时 `patterns` 移除 `macOS/*.app`(`Get-FileHash` 不能 hash 目录,且 GitHub release 实际分发的是 `.dmg`),`Get-ChildItem` 加 `-File` 过滤。

### 6. 重打 v0.1.0 tag

修复都在 main 后续 commit 里,但 `v0.1.0` tag 指向的是修复前的旧 commit。release workflow trigger 包含 `tags: ["v*"]`,所以必须重打 tag 才能让新代码进 release:

```bash
git tag -d v0.1.0
git push origin --delete v0.1.0
git tag v0.1.0                  # 默认指向当前 HEAD
git push origin v0.1.0
```

**tag 是不可变快照,修复代码后必须显式重打或打新 tag(v0.1.1),push 后续 commit 不会自动更新 tag**。

## Why This Works

- **裸二进制 vs 压缩包**:eugeneware release 直接给 ffmpeg 可执行文件,`download_to` 即可;BtbN release 给带目录结构的压缩包,必须 `tar`+`xz2` 或 `zip` 解压并精确匹配 `bin/ffmpeg[.exe]`,否则会把 `ffprobe` 或 `LICENSE` 误当成目标
- **校验粒度不可降级**:BtbN 公布只有压缩包 hash,强行校验解压后 ffmpeg 要么写死预期值(失同步)要么放弃校验;直接校验压缩包是唯一可靠路径
- **enum 调度统一主函数**:`fetch_and_prepare` 一个 `match` 走两个分支,避免主流程出现 `if windows { ... } else if macos { ... }` 这种平台判断散落各处的写法
- **macOS 兜底 eugeneware 是现实约束**:BtbN 不提供 macOS build 是上游事实,不是设计取舍
- **Linux 系统依赖的 apt 列表与 Tauri 2 官方一致**:`libwebkit2gtk-4.1-dev` + `libgtk-3-dev` 是 Tauri 2 webkit 后端在 Linux 上的运行时+链接时依赖
- **PowerShell 表达式赋值自动收集数组**:`$entries = foreach { ... }` 比 `+= $_` 高效且无 parser 问题
- **tag 是不可变引用**:Git tag 一旦 push 出去就不随后续 commit 改变,release workflow 用 tag 触发,必须在代码修复后**重打或打新 tag**

## Prevention

- **锁 ffmpeg 版本**:当前 `BTBN_VERSION = "latest"` 保留灵活性,但生产发布建议改具体 commit/tag(例如 `autobuild-2026-06-20-12-50`),避免 latest 漂移造成不可重现的构建
- **CI cache BtbN 压缩包**:key 用 `btbn-{asset_stem}-{checksums sha}`,既保留强制重下 BtbN 的正确性,又让 CI 不每次都打满 GitHub 带宽(~80MB × 4 平台 × N build)
- **BtbN latest 健康度监测**:BtbN 是每日构建,某天可能出 broken build;建议在 release job 末尾跑 `ffmpeg -version` smoke test,挂掉直接 fail CI
- **asset 名映射抽常量**:目前 `asset` / `asset_stem` 字符串散在 `match` 分支,建议提到文件顶部 `const` 区,eugeneware/BtbN 改名时只改一处
- **PowerShell 跨平台 PR 模板提醒**:`Get-ChildItem` 必须加 `-File` 过滤目录(`.app` / `.AppImage` 等),跨平台 PR 模板 checklist 加一条
- **tag 修复 SOP**:发现 release 包含的代码有 bug 时,先 `git tag -d` + `git push origin --delete`,再 `git tag <ver> && git push origin <ver>` 到新 HEAD;不要在原 tag 上 force push(Git 拒绝)
- **BtbN 解压过滤升级**:当前 `name.contains("/bin/")` 在 tar 路径是 `bin/`,zip 路径是 `ffmpeg-master-latest-linux64-gpl/bin/`,BtbN 未来加 `bin/ffmpeg-debug` 时需要 `name.ends_with("/bin/ffmpeg")` 这种带前导斜杠的更严格匹配

## Related

- `docs/solutions/build-errors/` 同目录其他 CI / build 修复
- Tauri 2 Linux 系统依赖官方文档:https://v2.tauri.app/start/prerequisites/#linux
- BtbN/FFmpeg-Builds release:https://github.com/BtbN/FFmpeg-Builds/releases/tag/latest
- eugeneware/ffmpeg-static release:https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.1.1
