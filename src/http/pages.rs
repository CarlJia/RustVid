use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
};

use crate::app::AppState;

pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let jobs = state.db.list_jobs().unwrap_or_default();
    let usage = state.capacity.usage().ok();
    let mut rows = String::new();
    for job in jobs {
        let action = job
            .artifact_id
            .as_ref()
            .map(|id| {
                format!(
                    r#"<a href="/jobs/{job_id}">查看</a> <a href="/assets/{id}/download">下载</a>"#,
                    job_id = job.id
                )
            })
            .unwrap_or_else(|| format!(r#"<a href="/jobs/{}">查看</a>"#, job.id));
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{:?}</td><td>{:?}</td><td>{:?}</td><td>{}</td></tr>",
            job.id, job.preset, job.target, job.status, action
        ));
    }
    let usage_text = usage
        .map(|u| {
            format!(
                "已用 {} MB / 上限 {} GB",
                u.used_bytes / 1024 / 1024,
                u.max_bytes / 1024 / 1024 / 1024
            )
        })
        .unwrap_or_else(|| "容量信息暂不可用".to_string());
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>RustVid 视频处理后台</title>
  <link rel="stylesheet" href="/static/styles.css">
</head>
<body>
  <main class="shell">
    <header>
      <h1>RustVid 视频处理后台</h1>
      <p>上传视频，选择用途预设，导出 MP4 或 HLS/m3u8 文件包。</p>
      <strong>{usage_text}</strong>
    </header>
    <section class="panel">
      <h2>上传并创建任务</h2>
      <form id="upload-form">
        <label>选择视频 <input type="file" id="file" required></label>
        <label>用途预设
          <select id="preset">
            <option value="blog">博客发布 - 720p 中等码率</option>
            <option value="course">课程播放 - 1080p 稳定清晰</option>
            <option value="mobile">移动端优先 - 540p 较低码率</option>
            <option value="archive">高清留档 - 1080p 较高质量</option>
          </select>
        </label>
        <fieldset>
          <legend>输出目标</legend>
          <label><input type="radio" name="target" value="mp4" checked> MP4（推荐）</label>
          <label><input type="radio" name="target" value="hls"> HLS/m3u8（高级发布）</label>
        </fieldset>
        <button type="submit">上传并创建任务</button>
      </form>
      <p id="message"></p>
    </section>
    <section class="panel">
      <h2>历史任务</h2>
      <table>
        <thead><tr><th>任务</th><th>预设</th><th>目标</th><th>状态</th><th>操作</th></tr></thead>
        <tbody>{rows}</tbody>
      </table>
    </section>
  </main>
  <script src="/static/app.js"></script>
</body>
</html>"#
    ))
}

pub async fn job_page(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let job = state.db.get_job(&id).ok().flatten();
    let Some(job) = job else {
        return Html("<h1>任务不存在</h1>".to_string());
    };
    let preview = job.artifact_id.as_ref().map(|artifact_id| {
        if job.target == crate::domain::preset::OutputTarget::Mp4 {
            format!(r#"<video controls src="/assets/{artifact_id}/preview"></video>"#)
        } else {
            format!(r#"<p>HLS 预览地址：<a href="/assets/{artifact_id}/preview">stream.m3u8</a></p>"#)
        }
    }).unwrap_or_else(|| "任务还没有可预览产物。".to_string());
    let download = job
        .artifact_id
        .as_ref()
        .map(|artifact_id| {
            format!(r#"<a class="button" href="/assets/{artifact_id}/download">下载产物</a>"#)
        })
        .unwrap_or_default();
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>任务详情</title>
  <link rel="stylesheet" href="/static/styles.css">
</head>
<body>
  <main class="shell">
    <a href="/">返回首页</a>
    <section class="panel">
      <h1>任务详情</h1>
      <p>状态：{:?}</p>
      <p>输出：{:?}</p>
      <p>{}</p>
      {}
      {}
    </section>
  </main>
</body>
</html>"#,
        job.status,
        job.target,
        job.error_summary.unwrap_or_else(|| "暂无错误".to_string()),
        preview,
        download,
    ))
}
