const form = document.querySelector("#upload-form");
const message = document.querySelector("#message");

function setMessage(text) {
  if (message) message.textContent = text;
}

async function uploadFile(file) {
  const create = await fetch("/api/uploads", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ filename: file.name, length: file.size }),
  });
  if (!create.ok) throw new Error((await create.json()).error || "创建上传失败");
  const upload = await create.json();

  const chunkSize = 8 * 1024 * 1024;
  let offset = upload.offset;
  while (offset < file.size) {
    const chunk = file.slice(offset, Math.min(offset + chunkSize, file.size));
    const patch = await fetch(`/api/uploads/${upload.id}`, {
      method: "PATCH",
      headers: {
        "content-type": "application/offset+octet-stream",
        "Upload-Offset": String(offset),
      },
      body: chunk,
    });
    if (!patch.ok) throw new Error((await patch.json()).error || "上传分片失败");
    offset = Number(patch.headers.get("Upload-Offset"));
    setMessage(`上传中：${Math.round((offset / file.size) * 100)}%`);
  }
  return upload.id;
}

async function createJob(uploadId) {
  const preset = document.querySelector("#preset").value;
  const target = document.querySelector("input[name='target']:checked").value;
  const response = await fetch("/api/jobs", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ upload_id: uploadId, preset, target }),
  });
  if (!response.ok) throw new Error((await response.json()).error || "创建任务失败");
  return response.json();
}

if (form) {
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const file = document.querySelector("#file").files[0];
    if (!file) return;
    try {
      setMessage("开始上传...");
      const uploadId = await uploadFile(file);
      setMessage("上传完成，正在创建转码任务...");
      const job = await createJob(uploadId);
      setMessage("任务已创建，正在请求处理...");
      await fetch("/api/jobs/process-next", { method: "POST" });
      window.location.href = `/jobs/${job.id}`;
    } catch (error) {
      setMessage(error.message);
    }
  });
}
