//! 打印调度：向真实打印机队列提交作业并轮询等待完成。调用系统工具 lp / lpstat / cancel。

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::i18n::{self, Lang};

fn t(key: &str) -> String {
    i18n::texts(Lang::from_env())
        .get(key)
        .unwrap_or(key)
        .to_string()
}

fn t_fmt(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// 提交一个打印作业，返回 CUPS 作业 ID（如 "HP_LaserJet_1020-42"）
pub fn submit(
    real_printer: &str,
    title: &str,
    copies: u32,
    extra_options: &[(String, String)],
    file: &Path,
) -> Result<String> {
    let mut cmd = Command::new("lp");
    cmd.arg("-d").arg(real_printer).arg("-t").arg(title);
    if copies > 1 {
        cmd.arg("-n").arg(copies.to_string());
    }
    for (k, v) in extra_options {
        cmd.arg("-o").arg(format!("{k}={v}"));
    }
    cmd.arg(file);

    let out = cmd.output().context(t("print_lp_failed"))?;
    if !out.status.success() {
        bail!(t_fmt(
            "print_lp_submit_failed",
            &[("msg", String::from_utf8_lossy(&out.stderr).trim())]
        ));
    }
    // 标准输出形如：request id is HP_LaserJet_1020-42 (1 file(s))
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .split_whitespace()
        .nth(3)
        .map(str::to_string)
        .with_context(|| t_fmt("print_lp_parse_failed", &[("msg", stdout.trim())]))
}

/// 等待作业离开队列（打印完成或被取消）。返回 Ok(true)=完成, Ok(false)=超时。
pub fn wait_done(
    real_printer: &str,
    job_id: &str,
    timeout: Duration,
    poll: Duration,
) -> Result<bool> {
    let start = Instant::now();
    loop {
        let out = Command::new("lpstat")
            .args(["-o", real_printer])
            .output()
            .context(t("print_lpstat_failed"))?;
        if out.status.success() {
            let queue = String::from_utf8_lossy(&out.stdout);
            if !queue.lines().any(|l| l.starts_with(job_id)) {
                return Ok(true);
            }
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        sleep(poll);
    }
}

/// 取消一个作业（用于出错清理）
pub fn cancel(job_id: &str) -> Result<()> {
    let out = Command::new("cancel")
        .arg(job_id)
        .output()
        .context(t("print_cancel_failed"))?;
    if !out.status.success() {
        bail!(t_fmt(
            "print_cancel_err",
            &[("msg", String::from_utf8_lossy(&out.stderr).trim())]
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_lp_stdout() {
        let line = "request id is HP_LaserJet_1020-42 (1 file(s))";
        assert_eq!(line.split_whitespace().nth(3), Some("HP_LaserJet_1020-42"));
    }
}
