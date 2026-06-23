/**
 * Tauri command 封装层
 *
 * 每个函数对应一个 `#[tauri::command]`,通过 `invoke()` 调到 Rust 后端。
 * 命名约定:TS 用 camelCase,Tauri 自动映射到 Rust 的 snake_case。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

// ---- 类型定义(对应 Rust struct) ----

export interface Preset {
  id: string;
  name: string;
  summary: string;
  resolution: string;
  bitrate_hint: string;
  hls_hint: string;
}

export interface UploadInfo {
  id: string;
  filename: string;
  length: number;
  offset: number;
  status: string;
}

export interface JobInfo {
  id: string;
  upload_id: string;
  /** 源上传文件名(后端从 uploads 表查得,旧任务可能为 null) */
  upload_filename: string | null;
  preset: string;
  /** 中文可读预设名("博客发布" 等),替代原始 id */
  preset_label: string;
  /** 预设一句话描述("适合个人博客和普通网页嵌入") */
  preset_summary: string;
  target: string;
  /** 中文可读输出目标("MP4 视频" / "HLS 流媒体") */
  target_label: string;
  status: string;
  error_summary: string | null;
  artifact_id: string | null;
  /** 源视频总时长(秒);null = 探测失败或旧任务 */
  source_duration_secs: number | null;
  /** 创建时间(ISO 8601 字符串) */
  created_at: string;
}

export interface UsageInfo {
  /** RustVid 已用字节数(uploads + artifacts) */
  used_bytes: number;
  /** 系统磁盘剩余可用字节数(data_dir 所在文件系统) */
  disk_free_bytes: number;
  /** 系统磁盘总容量字节数(供进度条展示) */
  disk_total_bytes: number;
}

/** Rust 端 emit 的实时进度事件 payload */
export interface TranscodeProgressEvent {
  job_id: string;
  encoded_secs: number;
  speed: number;
  /** 0-100;null = 源时长未知(没法算百分比) */
  percent: number | null;
}

// ---- 文件选择 ----

export async function pickVideoFile(): Promise<string | null> {
  const result = await openDialog({
    multiple: false,
    filters: [
      { name: "Video", extensions: ["mp4", "mov", "avi", "mkv", "webm", "m4v"] },
    ],
  });
  if (result == null) return null;
  if (Array.isArray(result)) return result[0] ?? null;
  return result;
}

// ---- Command 包装 ----

export const getPresets = (): Promise<Preset[]> => invoke("get_presets");

export const getUsage = (): Promise<UsageInfo> => invoke("get_usage");

export const createUpload = (
  filename: string,
  path: string,
): Promise<UploadInfo> => invoke("create_upload", { filename, path });

export const listJobs = (): Promise<JobInfo[]> => invoke("list_jobs");

export const getJob = (id: string): Promise<JobInfo | null> =>
  invoke("get_job", { id });

export const createJob = (
  uploadId: string,
  preset: string,
  target: string,
): Promise<JobInfo> => invoke("create_job", { uploadId, preset, target });

export const deleteJob = (id: string): Promise<void> =>
  invoke("delete_job", { id });

/** 一键清理所有失败任务(文件 + DB 行),返回清理数量 */
export const deleteFailedJobs = (): Promise<number> =>
  invoke("delete_failed_jobs");

export const retryJob = (id: string): Promise<JobInfo> =>
  invoke("retry_job", { id });

export const processNext = (): Promise<boolean> => invoke("process_next");

export const getArtifactPreviewPath = (id: string): Promise<string> =>
  invoke("get_artifact_preview_path", { id });

export const getArtifactDownloadPath = (id: string): Promise<string> =>
  invoke("get_artifact_download_path", { id });

export const revealInFinder = (path: string): Promise<void> =>
  invoke("reveal_in_finder", { path });

/** 订阅转码进度事件,返回取消订阅函数 */
export const onTranscodeProgress = (
  callback: (event: TranscodeProgressEvent) => void,
): Promise<UnlistenFn> => listen<TranscodeProgressEvent>("transcode-progress", (e) => callback(e.payload));

/** 下载产物:服务端把 artifact 复制到 dest,返回最终保存路径 */
export const downloadArtifact = (id: string, dest: string): Promise<string> =>
  invoke("download_artifact", { id, dest });
