//! CUPS backend main flow.
//!
//! Called by cupsd as: `dupriman <job-id> <user> <title> <copies> <options> [file]`
//! When `file` is omitted, reads print data from stdin.
//!
//! Flow:
//! 1. Simplex -> forward directly to the real printer
//! 2. Duplex -> split odd/even pages -> print all fronts (all copies)
//!    -> wait for physical completion -> request user confirmation
//!    -> print all backs
//!
//! Exit codes follow CUPS backend convention: 0 = success, non-zero = failure
//! (stderr goes to /var/log/cups/error_log).

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::Config;
use crate::options::{self, DuplexMode};
use crate::pdf;
use crate::print;
use crate::prompt_proto::{self, PromptRequest, WaitOutcome};

pub struct BackendArgs {
    pub job_id: String,
    pub user: String,
    pub title: String,
    pub copies: u32,
    pub options: String,
    pub file: Option<PathBuf>,
}

impl BackendArgs {
    /// Parse CUPS backend arguments (excluding argv[0])
    pub fn from_cups_args(args: &[String]) -> Result<Self> {
        if args.len() < 5 {
            bail!(
                "Insufficient CUPS backend arguments (expected 5 or 6, got {})",
                args.len()
            );
        }
        Ok(Self {
            job_id: args[0].clone(),
            user: args[1].clone(),
            title: args[2].clone(),
            copies: args[3].parse().context("copies is not a number")?,
            options: args[4].clone(),
            file: args.get(5).map(PathBuf::from),
        })
    }
}

fn log(msg: &str) {
    // CUPS backend stderr goes to /var/log/cups/error_log
    eprintln!("INFO: [dupriman] {msg}");
}

pub fn run(cfg: &Config, args: BackendArgs) -> i32 {
    match run_inner(cfg, &args) {
        Ok(()) => {
            log(&cfg.prompt.cli("cli_job_completed"));
            0
        }
        Err(e) => {
            eprintln!("ERROR: [dupriman] {e:#}");
            1
        }
    }
}

fn run_inner(cfg: &Config, args: &BackendArgs) -> Result<()> {
    log(&format!(
        "Job #{} user={} title={:?} copies={} options={:?}",
        args.job_id, args.user, args.title, args.copies, args.options
    ));

    fs::create_dir_all(&cfg.paths.spool_dir)
        .with_context(|| format!("Cannot create spool directory {}", cfg.paths.spool_dir.display()))?;

    // 读取输入（文件参数或 stdin）
    let input = match &args.file {
        Some(f) => f.clone(),
        None => {
            let tmp = cfg
                .paths
                .spool_dir
                .join(format!("job-{}-input.pdf", args.job_id));
            let mut data = Vec::new();
            io::stdin().read_to_end(&mut data).context(cfg.prompt.cli("backend_stdin_read_failed"))?;
            fs::write(&tmp, data)?;
            tmp
        }
    };
    let _guard = TempGuard::new(cfg, &input, args.file.is_none());

    let total = pdf::page_count(&input).context(
        "Cannot parse input as PDF. The virtual printer only accepts PDF (print via CUPS queue, do not send other formats directly)",
    )?;
    if total == 0 {
        bail!("Document has no pages");
    }

    let opts = options::parse(&args.options);
    log(&format!("Total {total} pages, mode: {}", cfg.prompt.cli(match opts.mode {
        DuplexMode::Simplex => "duplex_simplex",
        DuplexMode::LongEdge => "duplex_long_edge",
        DuplexMode::ShortEdge => "duplex_short_edge",
    })));

    let mut forward: Vec<(String, String)> = cfg
        .print
        .forward_options
        .iter()
        .filter_map(|k| opts.get(k).map(|v| (k.clone(), v.to_string())))
        .collect();
    forward.push(("sides".into(), "one-sided".into()));

    match opts.mode {
        DuplexMode::Simplex => print_and_wait(cfg, args, &input, "", &forward, total),
        mode => run_duplex(cfg, args, &input, total, mode, &forward),
    }
}

fn run_duplex(
    cfg: &Config,
    args: &BackendArgs,
    input: &std::path::Path,
    total: u32,
    mode: DuplexMode,
    forward: &[(String, String)],
) -> Result<()> {
    let odds = pdf::odd_pages(total);
    let (even_order, even_rotate) = cfg.duplex.for_mode(mode);
    let evens = pdf::even_pages(total, even_order);

    // Single page has no back side, treat as simplex
    if evens.is_empty() {
        log(&cfg.prompt.cli("cli_single_page"));
        return print_and_wait(cfg, args, input, "", forward, total);
    }

    let odd_pdf = cfg
        .paths
        .spool_dir
        .join(format!("job-{}-front.pdf", args.job_id));
    let even_pdf = cfg
        .paths
        .spool_dir
        .join(format!("job-{}-back.pdf", args.job_id));
    let blank_pdf = cfg
        .paths
        .spool_dir
        .join(format!("job-{}-blank.pdf", args.job_id));
    let _guard = TempGuard::new(cfg, &odd_pdf, true);
    let _guard2 = TempGuard::new(cfg, &even_pdf, true);
    let _guard3 = TempGuard::new(cfg, &blank_pdf, true);

    // Fronts: all odd pages × all copies
    pdf::extract(input, &odds, &odd_pdf)?;
    log(&cfg.prompt.cli_fmt(
        "cli_printing_fronts",
        &[("pages", &odds.len().to_string()), ("copies", &args.copies.to_string())],
    ));
    print_and_wait(cfg, args, &odd_pdf, " (front)", forward, total)?;

    // Prompt for confirmation
    let has_blank = total % 2 == 1;
    if cfg.prompt.enabled {
        let detail = cfg.prompt.resolved_detail(
            &args.job_id,
            &args.title,
            total,
            evens.len(),
            has_blank,
        );
        confirm_or_abort(cfg, args, &detail)?;
    } else {
        log(&cfg.prompt.cli("cli_prompt_disabled"));
    }

    // Backs: even pages, rotated/reordered as configured
    pdf::extract(input, &evens, &even_pdf)?;
    // Odd-page document: append a blank page so the last sheet also goes through
    // the back-side pass, allowing the user to collect all paper at once.
    // Since backs are already reversed, the blank page is placed first — after
    // reversal it prints last and lands on the back of the bottom sheet (page 1).
    if has_blank {
        pdf::blank_page(&blank_pdf)?;
        let merged = cfg
            .paths
            .spool_dir
            .join(format!("job-{}-back-full.pdf", args.job_id));
        let _guard4 = TempGuard::new(cfg, &merged, true);
        pdf::merge(&blank_pdf, &even_pdf, &merged)?; // blank page first
        pdf::rotate_all(&merged, even_rotate)?;
        log(&cfg.prompt.cli_fmt(
            "cli_printing_backs_blank",
            &[("pages", &evens.len().to_string()), ("copies", &args.copies.to_string())],
        ));
        return print_and_wait(cfg, args, &merged, " (back)", forward, total);
    }
    pdf::rotate_all(&even_pdf, even_rotate)?;
    log(&cfg.prompt.cli_fmt(
        "cli_printing_backs",
        &[("pages", &evens.len().to_string()), ("copies", &args.copies.to_string())],
    ));
    print_and_wait(cfg, args, &even_pdf, " (back)", forward, total)
}

/// Request user confirmation; abort job if cancelled or timed out
fn confirm_or_abort(cfg: &Config, args: &BackendArgs, detail: &str) -> Result<()> {
    let id = prompt_proto::make_request_id(&args.job_id);
    let req = PromptRequest {
        id: id.clone(),
        job_id: args.job_id.clone(),
        user: args.user.clone(),
        title: args.title.clone(),
        detail: detail.to_string(),
    };
    log(&cfg.prompt.cli("cli_waiting_confirmation"));
    prompt_proto::send_request(&cfg.paths.state_dir, &req)?;
    let timeout = (cfg.prompt.timeout_secs > 0).then(|| Duration::from_secs(cfg.prompt.timeout_secs));
    let outcome = prompt_proto::wait_response(
        &cfg.paths.state_dir,
        &id,
        timeout,
        Duration::from_millis(500),
    )?;
    match outcome {
        WaitOutcome::Confirmed => {
            log(&cfg.prompt.cli("cli_user_confirmed"));
            Ok(())
        }
        WaitOutcome::Declined(reason) => {
            bail!(cfg.prompt.cli_fmt("backend_user_cancelled", &[("reason", &reason)]))
        }
        WaitOutcome::Timeout => {
            bail!(cfg.prompt.cli("backend_timeout"))
        }
    }
}

/// 提交作业并等待真实打印完成。
///
/// 若 `cfg.printer.real` 为特殊值 `"save-pdf"`，则不调用真实打印机，
/// 而是把子作业 PDF 复制到 `spool_dir/saved/` 并立即视为完成——
/// 用于测试/调试，不消耗纸张。
fn print_and_wait(
    cfg: &Config,
    args: &BackendArgs,
    file: &std::path::Path,
    suffix: &str,
    forward: &[(String, String)],
    total: u32,
) -> Result<()> {
    let title = format!("{}{}", args.title, suffix);

    // Test mode: save PDF instead of printing
    if cfg.printer.real == "save-pdf" {
        let out_dir = cfg.paths.spool_dir.join("saved");
        fs::create_dir_all(&out_dir)
            .with_context(|| format!("Cannot create save directory {}", out_dir.display()))?;
        let dest = out_dir.join(format!(
            "job-{}{}-{}.pdf",
            args.job_id,
            suffix.replace([' ', '(', ')'], "-"),
            args.copies
        ));
        fs::copy(file, &dest).with_context(|| format!("Cannot save PDF to {}", dest.display()))?;
        log(&cfg.prompt.cli_fmt(
            "cli_saved_test",
            &[("path", &dest.display().to_string()), ("total", &total.to_string())],
        ));
        return Ok(());
    }

    let job = print::submit(&cfg.printer.real, &title, args.copies, forward, file)?;
    log(&cfg.prompt.cli_fmt(
        "cli_submitted",
        &[("printer", &cfg.printer.real), ("job", &job), ("total", &total.to_string())],
    ));
    let ok = print::wait_done(
        &cfg.printer.real,
        &job,
        Duration::from_secs(cfg.print.job_timeout_secs),
        Duration::from_millis(cfg.print.poll_interval_ms),
    )?;
    if !ok {
        print::cancel(&job).ok();
        bail!(cfg.prompt.cli_fmt("backend_print_timeout", &[("job", &job)]));
    }
    Ok(())
}

/// 临时文件清理守卫（keep_temp 时不删）
struct TempGuard<'a> {
    cfg: &'a Config,
    path: PathBuf,
    owned: bool,
}

impl<'a> TempGuard<'a> {
    fn new(cfg: &'a Config, path: &std::path::Path, owned: bool) -> Self {
        Self {
            cfg,
            path: path.to_path_buf(),
            owned,
        }
    }
}

impl Drop for TempGuard<'_> {
    fn drop(&mut self) {
        if self.owned && !self.cfg.print.keep_temp {
            fs::remove_file(&self.path).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cups_args_parse() {
        let args: Vec<String> = ["12", "alice", "报告", "2", "Duplex=DuplexNoTumble", "/tmp/f.pdf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = BackendArgs::from_cups_args(&args).unwrap();
        assert_eq!(parsed.job_id, "12");
        assert_eq!(parsed.copies, 2);
        assert!(parsed.file.is_some());

        let nofile: Vec<String> = ["12", "alice", "报告", "1", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(BackendArgs::from_cups_args(&nofile).unwrap().file.is_none());
        assert!(BackendArgs::from_cups_args(&nofile[..3]).is_err());
    }
}
