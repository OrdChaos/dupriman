//! 后端与弹窗守护进程的通信协议。
//!
//! CUPS 后端以 lp 用户运行、不在桌面会话中，无法直接弹 GTK 窗口。
//! 因此通过状态目录中的文件通信：
//! - 后端原子写入 `req-<id>.json`
//! - 守护进程弹窗后写入 `resp-<id>.json` 并删除请求文件
//! - 后端轮询响应，带超时；读完后删除响应文件

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    /// 请求唯一 ID（文件名组成部分）
    pub id: String,
    /// CUPS 作业号
    pub job_id: String,
    /// 提交作业的用户名（守护进程只响应同名用户的请求）
    pub user: String,
    /// 文档标题
    pub title: String,
    /// 附加信息（如页数统计），追加显示在提示文案后
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub id: String,
    /// true = 用户点击确定，继续打印背面
    pub confirmed: bool,
    /// 取消原因（confirmed=false 时）
    pub reason: String,
}

/// 等待结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    Confirmed,
    Declined(String),
    Timeout,
}

pub fn request_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("req-{id}.json"))
}

pub fn response_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("resp-{id}.json"))
}

/// 原子写入请求文件（先写临时文件再 rename，避免守护进程读到半截）
///
/// 注意：CUPS 后端以 cups 用户运行且 umask 严格（默认 0600），
/// 必须显式 chmod 0644，否则用户态守护进程读不到请求。
pub fn send_request(dir: &Path, req: &PromptRequest) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("无法创建状态目录 {}", dir.display()))?;
    let data = serde_json::to_vec_pretty(req)?;
    let tmp = dir.join(format!(".tmp-{}", req.id));
    fs::write(&tmp, data)?;
    make_world_readable(&tmp)?;
    fs::rename(&tmp, request_path(dir, &req.id))?;
    Ok(())
}

/// 设置文件为 0644，让状态目录另一侧的进程（不同用户）可读
fn make_world_readable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
        .with_context(|| format!("无法设置文件权限 {}", path.display()))
}

/// 等待响应。`timeout` 为 None 表示无限等待。
pub fn wait_response(
    dir: &Path,
    id: &str,
    timeout: Option<Duration>,
    poll: Duration,
) -> Result<WaitOutcome> {
    let start = Instant::now();
    let resp_path = response_path(dir, id);
    loop {
        if let Ok(data) = fs::read(&resp_path)
            && let Ok(resp) = serde_json::from_slice::<PromptResponse>(&data)
        {
            cleanup(dir, id);
            return Ok(if resp.confirmed {
                WaitOutcome::Confirmed
            } else {
                WaitOutcome::Declined(resp.reason)
            });
        }
        if let Some(t) = timeout
            && start.elapsed() >= t
        {
            cleanup(dir, id);
            return Ok(WaitOutcome::Timeout);
        }
        sleep(poll);
    }
}

/// 清理请求/响应/标记文件（后端在收到响应后调用）
pub fn cleanup(dir: &Path, id: &str) {
    fs::remove_file(request_path(dir, id)).ok();
    fs::remove_file(response_path(dir, id)).ok();
    fs::remove_file(dir.join(format!(".done-{id}"))).ok();
}

/// 守护进程侧：列出待处理请求（按文件名排序保证顺序）。
/// 跳过已有 `.done-<id>` 标记的请求（防止重复弹窗）。
pub fn pending_requests(dir: &Path) -> Vec<(PathBuf, PromptRequest)> {
    let mut items: Vec<(PathBuf, PromptRequest)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("req-") && n.ends_with(".json"))
        })
        .filter(|p| {
            // 从 req-<id>.json 提取 id，检查是否已处理
            let id = p
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.trim_start_matches("req-").to_string())
                .unwrap_or_default();
            !is_done(dir, &id)
        })
        .filter_map(|p| {
            let req = serde_json::from_slice::<PromptRequest>(&fs::read(&p).ok()?).ok()?;
            Some((p, req))
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}

/// 守护进程侧：写入响应并**标记请求为已处理**。
///
/// 响应文件 chmod 0644（后端以 cups 用户读取）。
/// 因 1777 sticky 目录下非 owner 无法删除 `req-*` 文件（属 cups 用户），
/// 改为在请求文件同目录写入 `.done-<id>` 标记；`pending_requests` 跳过
/// 已有标记的请求。真正的删除由后端（cups 用户）在收到响应后执行，
/// 或交给 systemd-tmpfiles 定期清理。
pub fn send_response(dir: &Path, resp: &PromptResponse) -> Result<()> {
    let data = serde_json::to_vec_pretty(resp)?;
    let tmp = dir.join(format!(".tmp-resp-{}", resp.id));
    fs::write(&tmp, data)?;
    make_world_readable(&tmp)?;
    fs::rename(&tmp, response_path(dir, &resp.id))?;
    // 标记请求已处理，防止守护进程重复弹窗
    let done = dir.join(format!(".done-{}", resp.id));
    fs::write(&done, b"").ok();
    make_world_readable(&done).ok();
    Ok(())
}

/// 检查请求是否已被守护进程处理过
pub fn is_done(dir: &Path, id: &str) -> bool {
    dir.join(format!(".done-{id}")).exists()
}

/// 生成请求 ID：作业号 + 毫秒时间戳
pub fn make_request_id(job_id: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{job_id}-{ts}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_response_roundtrip() {
        let dir = std::env::temp_dir().join(format!("dupriman-proto-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let req = PromptRequest {
            id: "t1".into(),
            job_id: "42".into(),
            user: "alice".into(),
            title: "测试文档".into(),
            detail: "共 10 页".into(),
        };
        send_request(&dir, &req).unwrap();

        let pending = pending_requests(&dir);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.user, "alice");

        // 请求文件必须是其他用户可读的（CUPS 后端 cups 用户 -> 守护进程普通用户）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(request_path(&dir, "t1")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o644, "请求文件权限应为 0644，实际 {mode:o}");
        }

        send_response(
            &dir,
            &PromptResponse {
                id: "t1".into(),
                confirmed: false,
                reason: "测试取消".into(),
            },
        )
        .unwrap();
        assert!(pending_requests(&dir).is_empty());

        let outcome = wait_response(&dir, "t1", Some(Duration::from_secs(2)), Duration::from_millis(50)).unwrap();
        assert_eq!(outcome, WaitOutcome::Declined("测试取消".into()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wait_times_out() {
        let dir = std::env::temp_dir().join(format!("dupriman-proto-to-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let outcome = wait_response(&dir, "nobody", Some(Duration::from_millis(200)), Duration::from_millis(50)).unwrap();
        assert_eq!(outcome, WaitOutcome::Timeout);
        fs::remove_dir_all(&dir).ok();
    }
}
