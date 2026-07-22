//! DupriMan — 单面打印机的手动双面打印助手。
//!
//! 单一二进制包含全部角色：
//! - CUPS 后端（被 cupsd 以 `dupriman://` URI 调用，或无参数时输出发现信息）
//! - `dupriman prompt`：GTK 弹窗守护进程（用户会话内）
//! - `dupriman install/uninstall`：按 FHS 安装/卸载并注册虚拟打印机
//! - `dupriman test`：环境自检

mod backend;
mod config;
mod i18n;
mod install;
mod options;
mod pdf;
mod ppd;
mod print;
mod prompt_proto;
mod promptd;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::exit;

use config::Config;

#[derive(Parser)]
#[command(
    name = "dupriman",
    version,
    about = "手动双面打印助手（CUPS 虚拟打印机 + 确认弹窗）"
)]
struct Cli {
    /// 额外加载的配置文件（覆盖系统级与用户级）
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 以 CUPS 后端方式运行（一般由 cupsd 调用）
    Backend {
        /// job-id user title copies options [file]
        #[arg(required = true, num_args = 5..=6)]
        args: Vec<String>,
    },
    /// 启动弹窗守护进程（图形会话内，通常由 systemd --user 启动）
    Prompt,
    /// 安装并注册虚拟打印机（需要 root；打包用 --root/--no-activate）
    Install {
        /// 目标根目录（打包时传 pkgdir）
        #[arg(long, default_value = "/")]
        root: PathBuf,
        /// 只落盘文件，不执行 lpadmin/systemctl
        #[arg(long)]
        no_activate: bool,
        /// 只执行激活动作（文件已由包管理器安装）
        #[arg(long)]
        activate_only: bool,
        /// 覆盖已存在的配置文件
        #[arg(long)]
        force: bool,
        /// 只打印计划，不执行
        #[arg(long)]
        dry_run: bool,
    },
    /// 卸载虚拟打印机与相关文件
    Uninstall {
        #[arg(long, default_value = "/")]
        root: PathBuf,
        #[arg(long)]
        no_activate: bool,
        /// 连同二进制与配置一起删除
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// 环境自检：依赖工具、配置、打印机、服务状态
    Test,
    /// 打印生成的 PPD 到标准输出
    Ppd,
    /// 打印示例配置到标准输出
    SampleConfig,
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();

    // CUPS 直接调用后端：argv = dupriman job-id user title copies options [file]
    if (6..=7).contains(&raw.len())
        && !raw[1].is_empty()
        && raw[1].chars().all(|c| c.is_ascii_digit())
    {
        let cfg = load_config(None);
        let args = match backend::BackendArgs::from_cups_args(&raw[1..]) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("ERROR: [dupriman] {e:#}");
                exit(1);
            }
        };
        exit(backend::run(&cfg, args));
    }

    // 无参数：CUPS 后端发现模式（输出设备列表）
    if raw.len() == 1 {
        let cfg = load_config(None);
        println!(
            "direct \"{}\" \"DupriMan\" \"{}\" \"MFG:DupriMan;MDL:Manual Duplex;\"",
            cfg.printer.virtual_uri, cfg.printer.virtual_info
        );
        eprintln!("（以上是 CUPS 后端发现输出；日常使用请运行 dupriman --help）");
        return;
    }

    let cli = Cli::parse();
    let cfg = load_config(cli.config.as_deref());

    let code = match cli.command {
        Some(Cmd::Backend { args }) => match backend::BackendArgs::from_cups_args(&args) {
            Ok(a) => backend::run(&cfg, a),
            Err(e) => {
                eprintln!("参数错误: {e:#}");
                2
            }
        },
        Some(Cmd::Prompt) => match promptd::run(&cfg) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("[dupriman-prompt] 启动失败: {e:#}");
                1
            }
        },
        Some(Cmd::Install {
            root,
            no_activate,
            activate_only,
            force,
            dry_run,
        }) => match install::install(
            &cfg,
            &install::InstallOpts {
                root,
                no_activate,
                activate_only,
                force,
                dry_run,
            },
        ) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("安装失败: {e:#}");
                1
            }
        },
        Some(Cmd::Uninstall {
            root,
            no_activate,
            purge,
            dry_run,
        }) => match install::uninstall(
            &cfg,
            &install::InstallOpts {
                root,
                no_activate,
                dry_run,
                ..Default::default()
            },
            purge,
        )
        .map(|_| ())
        {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("卸载失败: {e:#}");
                1
            }
        },
        Some(Cmd::Test) => self_test(&cfg),
        Some(Cmd::Ppd) => {
            print!("{}", ppd::generate(&cfg));
            0
        }
        Some(Cmd::SampleConfig) => {
            print!("{}", Config::sample_toml());
            0
        }
        None => 0,
    };
    exit(code);
}

fn load_config(extra: Option<&std::path::Path>) -> Config {
    match Config::load(extra) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("警告：配置加载失败，使用默认值: {e:#}");
            Config::default()
        }
    }
}

/// 环境自检，返回退出码（0 = 全部通过）
fn self_test(cfg: &Config) -> i32 {
    let mut failed = 0;
    let mut check = |name: &str, ok: bool, hint: &str| {
        if ok {
            println!("  ✓ {name}");
        } else {
            println!("  ✗ {name} —— {hint}");
            failed += 1;
        }
    };

    println!("Dependencies:");
    for (tool, pkg) in [
        ("pdfinfo", "poppler"),
        ("qpdf", "qpdf"),
        ("lp", "cups"),
        ("lpstat", "cups"),
        ("lpadmin", "cups"),
    ] {
        check(tool, which(tool), &format!("please install {pkg}"));
    }

    println!("Configuration:");
    check(
        "Config file parses correctly",
        Config::load(None).is_ok(),
        "check /etc/dupriman/config.toml syntax",
    );

    println!("Printers:");
    let lpstat_out = std::process::Command::new("lpstat")
        .arg("-v")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    check(
        &format!("Real printer {} exists", cfg.printer.real),
        lpstat_out.contains(&cfg.printer.real),
        "run lpstat -v and update printer.real in config",
    );
    check(
        &format!("Virtual printer {} registered", cfg.printer.virtual_name),
        lpstat_out.contains(&cfg.printer.virtual_name),
        "run sudo dupriman install",
    );

    println!("Runtime:");
    let state_ok = std::fs::metadata(&cfg.paths.state_dir)
        .map(|m| m.is_dir())
        .unwrap_or(false);
    check(
        &format!("State directory {} exists", cfg.paths.state_dir.display()),
        state_ok,
        "run sudo dupriman install",
    );
    let svc = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "dupriman-prompt.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false);
    check(
        "Prompt daemon running",
        svc,
        "run systemctl --user start dupriman-prompt",
    );

    if failed == 0 {
        println!("\n{}", cfg.prompt.cli("cli_all_passed"));
        0
    } else {
        println!(
            "\n{}",
            cfg.prompt.cli_fmt("cli_checks_failed", &[("failed", &failed.to_string())])
        );
        1
    }
}

fn which(tool: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(tool).is_file()))
        .unwrap_or(false)
}
