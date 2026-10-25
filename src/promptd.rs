//! 弹窗守护进程：在用户桌面会话中运行（由所选会话管理器监管）。
//!
//! 轮询状态目录，发现后端发来的确认请求后弹出 GTK4 对话框，
//! 将用户选择写回响应文件。只处理与当前用户同名的请求，
//! 文案全部来自配置（含用户级覆盖）。

use anyhow::{Context, Result};
use gtk4::glib;
use std::cell::Cell;
use std::rc::Rc;

use crate::config::Config;
use crate::prompt_proto::{self, PromptRequest, PromptResponse};

pub fn run(cfg: &Config) -> Result<()> {
    ensure_display_env();
    gtk4::init().context(
        "GTK initialization failed: please run inside a graphical session (WAYLAND_DISPLAY/DISPLAY available)",
    )?;

    let current_user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    if !cfg
        .prompt
        .allowed_users
        .iter()
        .any(|user| user == &current_user)
    {
        anyhow::bail!("current user is not listed in prompt.allowed_users");
    }
    let state_dir = prompt_proto::user_dir(&cfg.paths.state_dir, &current_user)?;
    let state_dir_display = state_dir.display().to_string();
    let prompt = cfg.prompt.clone();
    let busy = Rc::new(Cell::new(false));

    glib::timeout_add_seconds_local(1, {
        let busy = Rc::clone(&busy);
        move || {
            if busy.get() {
                return glib::ControlFlow::Continue;
            }
            let pending = prompt_proto::pending_requests(&state_dir);
            let Some((_, req)) = pending.first().cloned() else {
                return glib::ControlFlow::Continue;
            };
            eprintln!(
                "[dupriman-prompt] {}",
                prompt.cli_fmt(
                    "cli_confirmation_request",
                    &[("job_id", &req.job_id), ("title", &req.title)]
                )
            );
            busy.set(true);
            glib::MainContext::default().spawn_local(show_dialog(
                state_dir.clone(),
                req,
                prompt.clone(),
                Rc::clone(&busy),
            ));
            glib::ControlFlow::Continue
        }
    });

    eprintln!(
        "[dupriman-prompt] {}",
        cfg.prompt
            .cli_fmt("cli_daemon_started", &[("dir", &state_dir_display)])
    );
    glib::MainLoop::new(None, false).run();
    Ok(())
}

/// 确保 GTK 能连上显示服务器。
///
/// 若 WAYLAND_DISPLAY/DISPLAY 均未设置（例如服务先于会话启动、
/// compositor 未做 import-environment），扫描 $XDG_RUNTIME_DIR
/// 下的 wayland-* socket 自动设置 WAYLAND_DISPLAY。
fn ensure_display_env() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        return;
    }
    let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") else {
        return;
    };
    let mut sockets: Vec<_> = std::fs::read_dir(&runtime_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter_map(|n| n.into_string().ok())
        .filter(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        .collect();
    sockets.sort();
    if let Some(name) = sockets.first() {
        // 此处无法访问 cfg，直接用英文（属诊断信息，不影响用户界面）
        eprintln!(
            "[dupriman-prompt] No display env var found, auto-detecting WAYLAND_DISPLAY={name}"
        );
        // SAFETY: setting process env var during single-threaded startup
        unsafe { std::env::set_var("WAYLAND_DISPLAY", name) };
    }
}

async fn show_dialog(
    state_dir: std::path::PathBuf,
    req: PromptRequest,
    prompt: crate::config::PromptCfg,
    busy: Rc<Cell<bool>>,
) {
    let body = if req.detail.is_empty() {
        prompt.resolved_message()
    } else {
        format!("{}\n\n{}", prompt.resolved_message(), req.detail)
    };
    let dialog = gtk4::AlertDialog::builder()
        .modal(true)
        .message(prompt.resolved_title())
        .detail(&body)
        .buttons([
            prompt.resolved_cancel_label().as_str(),
            prompt.resolved_ok_label().as_str(),
        ])
        .cancel_button(0)
        .default_button(1)
        .build();

    let (confirmed, reason) = match dialog.choose_future(None::<&gtk4::Window>).await {
        Ok(1) => (true, String::new()),
        Ok(_) => (false, "user clicked cancel".into()),
        Err(_) => (false, "dialog closed".into()),
    };

    let resp = PromptResponse {
        id: req.id.clone(),
        capability: req.capability.clone(),
        confirmed,
        reason,
    };
    if let Err(e) = prompt_proto::send_response(&state_dir, &resp) {
        eprintln!(
            "[dupriman-prompt] {}",
            prompt.cli_fmt("cli_failed_write_response", &[("error", &format!("{e:#}"))])
        );
    }
    busy.set(false);
}
