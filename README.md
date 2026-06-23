# RustVid

RustVid 是一个面向普通创作者和站点维护者的 Rust 桌面视频处理客户端，基于 **Tauri 2 + Vue 3**。

- 上传视频（系统文件选择器）
- 选择用途预设（Blog / Course / Mobile / Archive）
- 导出 MP4 或 HLS/m3u8 文件包
- 内置 ffmpeg 静态构建（构建时下载并嵌入到产物里），**不依赖系统是否安装 FFmpeg**

## 当前定位

- 单用户/管理员模式，不是多租户视频托管平台。
- 默认推荐 MP4 输出，HLS/m3u8 是高级发布选项。
- 跨平台桌面客户端（macOS / Windows / Linux），通过 Tauri 打包成原生应用。
- 代码注释、用户可见文案和项目文档使用中文；Rust 标识符、库名和协议名保留英文。

## 技术栈

- **后端**：Rust + Tauri 2 (`#[tauri::command]` IPC)
- **前端**：Vue 3 + Vite + TypeScript
- **转码**：ffmpeg（构建时从 `eugeneware/ffmpeg-static` 下载适配本平台的静态构建，`include_bytes!` 嵌入到产物里，运行时从临时目录提取）
- **存储**：SQLite（rusqlite）
- **打包**：Tauri CLI 产出 `.app` / `.dmg` / `.exe` / `.msi` / `.deb` / `.AppImage`

## 运行依赖（仅开发时）

- Rust 1.95+
- Node.js 18+（用于前端构建）
- macOS 需要 Xcode Command Line Tools（`xcode-select --install`）
- **运行时无 ffmpeg 依赖**——ffmpeg 静态构建已嵌入到 RustVid.app 里

## 功能特性

- 🎬 **桌面原生窗口**(Tauri 2 + Vue 3,跨平台打包 `.app` / `.exe` / `.msi` / `.deb` / `.AppImage`)
- 📁 **文件选择器**:`tauri-plugin-dialog` 调系统原生文件对话框
- 🔍 **智能码率协商**:ffmpeg -i 探测源视频,自动 cap 到 `min(预设目标, 源码率)`,防止低码率源转码后体积膨胀
- ⏱ **实时进度事件**:后端用 ffmpeg `-progress pipe:1` 流式输出 → `app.emit("transcode-progress")` → 前端 `listen()` 显示已编码时长 + 速度
- 📊 **存储用量** / 任务历史 / 任务详情 / 失败重试 / 手动删除
- 🎥 **MP4 预览**:`convertFileSrc` 把本地路径转 webview 可访问的 URL
- 🗂 **HLS/m3u8 zip 包**:含 playlist、ts 分片、README

## 默认限制

- 单文件上限：2GB（防止单文件过大占用过多内存/磁盘）
- 同时转码任务上限：2 个

总存储用量不再硬性封顶——前端展示**系统磁盘剩余空间**（通过 `fs2::free_space` 读 `data_dir` 所在文件系统），由用户自行决定何时清理历史任务。

数据目录：
- macOS: `~/Library/Application Support/RustVid/`
- 可通过 `RUSTVID_DATA_DIR` 环境变量覆盖

## 开发模式

```bash
# 1. 安装前端依赖（仅首次）
cd frontend
npm install
cd ..

# 2. 跑开发模式（前端 HMR + Tauri 窗口）
./frontend/node_modules/.bin/tauri dev
```

或用 `npx --yes @tauri-apps/cli@latest dev`（首次会下载 CLI）。

## 打包

```bash
# 仅产出二进制（不打包 .app），用于快速验证
./frontend/node_modules/.bin/tauri build --no-bundle

# 完整打包（产出 .app 到 target/release/bundle/macos/）
./frontend/node_modules/.bin/tauri build --bundles app
```

打包产物：
- `target/release/bundle/macos/RustVid.app`（macOS 14MB，含 45MB embedded ffmpeg）
- `target/release/bundle/dmg/RustVid_0.1.0_aarch64.dmg`（macOS DMG 安装包，加上 `--bundles dmg`）

## 跨平台构建

| 平台 | 指令 | 产物 |
|---|---|---|
| macOS (arm64) | `./frontend/node_modules/.bin/tauri build` | `.app` / `.dmg` |
| macOS (x86_64) | 在 x86_64 Mac 上跑同上 | 同上 |
| Windows | 在 Windows 上跑同上 | `.exe` / `.msi` |
| Linux | 在 Linux 上跑同上 | `.deb` / `.AppImage` |

ffmpeg 静态二进制由 `build.rs` 根据 `TARGET` 自动选择，无需手动配置。

### Windows 打包

> **跨平台限制**:macOS/Linux 上**不能**直接交叉编译到 Windows。Tauri 依赖 WebView2(Win)/WebKitGTK(Linux) 等平台 native 库,这些只能在 Windows 上链接。**必须在 Windows runner(本机或 CI)上构建**。

`.github/workflows/release.yml` 矩阵:
| Runner | Target | 产物 |
|---|---|---|
| `windows-latest` | `x86_64-pc-windows-msvc` | NSIS `.exe` + portable `.zip` |
| `macos-latest` | `aarch64-apple-darwin` | `.app` / `.dmg` |
| `macos-13` | `x86_64-apple-darwin` | `.app` / `.dmg` |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.deb` / `.AppImage` |

触发方式:
- push tag `v*`(如 `v0.1.0`)
- Actions 页面 → "Release" → "Run workflow"(手动)
- 产物自动作为 artifact 上传,含 `SHA256SUMS` 校验文件

本地试 Windows 打包(必须 Windows,或在 CI runner 上):
```bash
# Windows 上(MSVC,推荐)
rustup target add x86_64-pc-windows-msvc
cargo tauri build
```
产 `target/release/bundle/nsis/RustVid_0.1.0_x64-setup.exe` 和 `_portable.zip`。

#### Portable 单文件用法

Windows job 在 CI 上会自动做这件事:7z 解 NSIS 安装器 → 找主 exe → 把所在目录(含 DLL)打成 `RustVid_0.1.0_x64_portable.zip`。

用户使用:
1. 下载 `_portable.zip`
2. 解压到任意目录(如 `D:\Tools\RustVid\`)
3. 双击 `RustVid.exe` 即可运行——**无需安装、无需管理员权限、不写注册表**

> Portable 模式首次运行会下载 WebView2 bootstrapper(Win11 已自带,Win10 需联网下载一次)并静默安装,后续离线运行。

#### 校验产物

```bash
# 下载 SHA256SUMS 后
sha256sum -c SHA256SUMS
```
或者 Windows PowerShell:
```powershell
Get-FileHash .\RustVid_0.1.0_x64-setup.exe -Algorithm SHA256
# 对比 SHA256SUMS 文件里的对应行
```

#### Windows 配置要点(`tauri.conf.json`)

```json
"bundle": {
  "windows": {
    "nsis": {
      "installMode": "currentUser",   // 不需要管理员权限
      "compression": "lzma",          // 更小体积
      "languages": ["SimpChinese", "English"]
    },
    "webviewInstallMode": {
      "type": "downloadBootstrapper", // Win11 自带;Win10 首次自动装
      "silent": true
    },
    "minimumWebview2Version": "110.0.0"
  }
}
```

## 跳过 bundled ffmpeg（回退到系统 ffmpeg）

```bash
SKIP_BUNDLED_FFMPEG=1 ./frontend/node_modules/.bin/tauri build
```

适用于：
- 离线环境
- CI 缓存命中
- 临时调试

## 验证

```bash
cargo test                       # 24 个测试（22 单测 + 2 集成）
cargo clippy --all-targets -- -D warnings  # 0 警告
cargo build                      # 调试模式编译
```

CI 验证(`.github/workflows/ci.yml`):
- `cargo fmt --check`
- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- 前端 `npm run build` (vue-tsc + vite)

集成测试使用 fake 转码器验证任务状态流。真实 ffmpeg 验证应用 `cargo run` 启动 dev 窗口上传短视频。

## 架构

```
src/                       # Rust 后端
├── main.rs                # Tauri 入口
├── lib.rs                 # 模块声明
├── app.rs                 # AppState + setup_logging
├── config.rs              # 配置
├── commands/              # Tauri command 处理器
│   ├── uploads.rs         # create_upload
│   ├── jobs.rs            # create_job / list_jobs / get_job / ...
│   ├── presets.rs         # get_presets
│   ├── usage.rs           # get_usage
│   └── artifacts.rs       # 产物路径 + reveal_in_finder
├── domain/                # 纯数据类型
├── services/              # 业务逻辑
│   ├── ffmpeg.rs          # 视频转码
│   ├── ffmpeg_binary.rs   # bundled ffmpeg 提取
│   ├── ffmpeg_probe.rs    # ffmpeg -i 解析
│   ├── job_queue.rs       # 任务队列
│   ├── capacity.rs        # 容量管理
│   ├── artifact_store.rs  # 产物存储
│   └── upload_sessions.rs # 上传会话
└── persistence/           # SQLite

frontend/                   # Vue 3 前端
├── package.json
├── vite.config.ts
├── index.html
└── src/
    ├── main.ts            # Vue 入口
    ├── App.vue            # 根组件
    ├── router.ts          # vue-router
    ├── api.ts             # invoke() 封装
    ├── style.css
    ├── views/
    │   ├── HomeView.vue   # 上传 + 任务列表
    │   └── JobView.vue    # 任务详情 + 预览
    └── components/
        ├── UploadForm.vue
        ├── JobList.vue
        └── StatusBadge.vue

tauri.conf.json             # Tauri 配置
build.rs                    # ffmpeg 静态构建下载
Cargo.toml                  # Rust 依赖
```

## 开发历史

- 早期版本为 Axum + 服务端渲染 HTML + 原生 JS 的 Web 应用
- 现版本完全重写为 Tauri 2 + Vue 3 桌面客户端
- 详见 `/Users/leo/.claude/plans/jazzy-rolling-chipmunk.md`
