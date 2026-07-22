//! 安装/卸载：按 FHS 布局放置文件，注册 CUPS 虚拟打印机与 systemd --user 服务。
//!
//! 布局：
//!   /usr/bin/dupriman                          二进制本体
//!   /usr/lib/cups/backend/dupriman             CUPS 后端（指向本体的符号链接）
//!   /usr/share/cups/model/dupriman.ppd         虚拟打印机 PPD
//!   /etc/dupriman/config.toml                  系统级配置（已存在则不覆盖）
//!   /usr/lib/systemd/user/dupriman-prompt.service  弹窗守护服务
//!   /usr/lib/tmpfiles.d/dupriman.conf          状态/临时目录（1777）
//!
//! 打包（如 PKGBUILD）用 `--root <pkgdir> --no-activate` 仅落盘；
//! 再用 `--activate-only` 在 post_install 中完成注册。

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::ppd;

pub const BIN_DEST: &str = "usr/bin/dupriman";
pub const BACKEND_DEST: &str = "usr/lib/cups/backend/dupriman";
pub const PPD_DEST: &str = "usr/share/cups/model/dupriman.ppd";
pub const CONFIG_DEST: &str = "etc/dupriman/config.toml";
pub const UNIT_DEST: &str = "usr/lib/systemd/user/dupriman-prompt.service";
pub const TMPFILES_DEST: &str = "usr/lib/tmpfiles.d/dupriman.conf";
pub const I18N_DIR: &str = "usr/share/dupriman/i18n";

pub struct InstallOpts {
    /// 目标根（打包时传 pkgdir）
    pub root: PathBuf,
    /// 跳过 lpadmin / systemctl / 目录创建（仅落盘）
    pub no_activate: bool,
    /// 只做激活动作（文件已由包管理器放置）
    pub activate_only: bool,
    /// 已存在的配置文件也覆盖
    pub force: bool,
    /// 只打印计划不执行
    pub dry_run: bool,
}

impl Default for InstallOpts {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/"),
            no_activate: false,
            activate_only: false,
            force: false,
            dry_run: false,
        }
    }
}

fn systemd_unit() -> String {
    r#"[Unit]
Description=DupriMan manual duplex printing confirmation prompt daemon
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/dupriman prompt
Restart=on-failure
RestartSec=3

[Install]
WantedBy=graphical-session.target
"#
    .to_string()
}

fn tmpfiles_conf(cfg: &Config) -> String {
    // age 参数让 systemd-tmpfiles 定期清理残留的请求/响应/临时文件
    // （1777 sticky 目录中非 owner 无法删除，跨用户残留属正常）
    format!(
        "d {} 1777 root root 1h\nd {} 1777 root root 1d\n",
        cfg.paths.state_dir.display(),
        cfg.paths.spool_dir.display()
    )
}

fn under(root: &Path, rel: &str) -> PathBuf {
    if root == Path::new("/") {
        PathBuf::from("/").join(rel)
    } else {
        root.join(rel)
    }
}

struct Plan {
    actions: Vec<String>,
}

impl Plan {
    fn new() -> Self {
        Self { actions: vec![] }
    }
    fn note(&mut self, msg: impl Into<String>) {
        self.actions.push(msg.into());
    }
}

/// 读取 /proc/self/status 获取有效 UID（避免引入 libc 依赖）
fn euid() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

fn require_root(opts: &InstallOpts) -> Result<()> {
    if opts.root == Path::new("/") && !opts.dry_run && euid() != Some(0) {
        bail!("Installing to system directories requires root privileges, please use sudo (use --root for packaging)");
    }
    Ok(())
}

pub fn install(cfg: &Config, opts: &InstallOpts) -> Result<()> {
    require_root(opts)?;
    let mut plan = Plan::new();

    if !opts.activate_only {
        // 1. 二进制
        let exe = std::env::current_exe().context("无法定位当前可执行文件")?;
        let bin = under(&opts.root, BIN_DEST);
        plan.note(format!("Installing binary {} -> {}", exe.display(), bin.display()));
        run(opts, &mut plan, || {
            ensure_parent(&bin)?;
            fs::copy(&exe, &bin)?;
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755))?;
            Ok(())
        })?;

        // 2. CUPS 后端符号链接
        let backend = under(&opts.root, BACKEND_DEST);
        plan.note(format!("Creating backend symlink {} -> /{}", backend.display(), BIN_DEST));
        run(opts, &mut plan, || {
            ensure_parent(&backend)?;
            fs::remove_file(&backend).ok();
            std::os::unix::fs::symlink(format!("/{BIN_DEST}"), &backend)?;
            Ok(())
        })?;

        // 3. PPD
        let ppd_path = under(&opts.root, PPD_DEST);
        plan.note(format!("Writing PPD {}", ppd_path.display()));
        run(opts, &mut plan, || {
            ensure_parent(&ppd_path)?;
            fs::write(&ppd_path, ppd::generate(cfg))?;
            Ok(())
        })?;

        // 4. 系统级配置（不存在才写）
        let conf = under(&opts.root, CONFIG_DEST);
        if conf.exists() && !opts.force {
            plan.note(format!("Config exists, skipping {} (use --force to overwrite)", conf.display()));
        } else {
            plan.note(format!("Writing config {}", conf.display()));
            run(opts, &mut plan, || {
                ensure_parent(&conf)?;
                fs::write(&conf, Config::sample_toml())?;
                Ok(())
            })?;
        }

        // 5. systemd --user 服务
        let unit = under(&opts.root, UNIT_DEST);
        plan.note(format!("Writing service {}", unit.display()));
        run(opts, &mut plan, || {
            ensure_parent(&unit)?;
            fs::write(&unit, systemd_unit())?;
            Ok(())
        })?;

        // 6. tmpfiles
        let tmpf = under(&opts.root, TMPFILES_DEST);
        plan.note(format!("Writing tmpfiles {}", tmpf.display()));
        run(opts, &mut plan, || {
            ensure_parent(&tmpf)?;
            fs::write(&tmpf, tmpfiles_conf(cfg))?;
            Ok(())
        })?;

        // 7. i18n locale files
        let i18n_dir = under(&opts.root, I18N_DIR);
        plan.note(format!("Installing i18n files to {}", i18n_dir.display()));
        run(opts, &mut plan, || {
            fs::create_dir_all(&i18n_dir)?;
            for stem in ["en_US", "zh_CN", "zh_TW"] {
                let content = match stem {
                    "en_US" => include_str!("../i18n/en_US.toml"),
                    "zh_CN" => include_str!("../i18n/zh_CN.toml"),
                    "zh_TW" => include_str!("../i18n/zh_TW.toml"),
                    _ => unreachable!(),
                };
                fs::write(i18n_dir.join(format!("{stem}.toml")), content)?;
            }
            Ok(())
        })?;
    }

    if opts.no_activate {
        plan.note("(--no-activate: skipping activation steps)".to_string());
    } else {
        activate(cfg, opts, &mut plan)?;
    }

    println!("{}", cfg.prompt.cli("cli_install_plan"));
    for a in &plan.actions {
        println!("  - {a}");
    }
    if !opts.dry_run && !opts.no_activate {
        println!("\n{}", cfg.prompt.cli("cli_done_confirm"));
    }
    Ok(())
}

fn activate(cfg: &Config, opts: &InstallOpts, plan: &mut Plan) -> Result<()> {
    // 状态/临时目录（等效 systemd-tmpfiles --create）
    plan.note(format!(
        "Creating directories {} and {} (1777)",
        cfg.paths.state_dir.display(),
        cfg.paths.spool_dir.display()
    ));
    run(opts, plan, || {
        for d in [&cfg.paths.state_dir, &cfg.paths.spool_dir] {
            fs::create_dir_all(d)?;
            fs::set_permissions(d, fs::Permissions::from_mode(0o1777))?;
        }
        Ok(())
    })?;

    // Register virtual printer
    plan.note(format!(
        "lpadmin registering virtual printer {} ({})",
        cfg.printer.virtual_name, cfg.printer.real
    ));
    run(opts, plan, || {
        let out = Command::new("lpadmin")
            .args(["-p", &cfg.printer.virtual_name])
            .args(["-v", &cfg.printer.virtual_uri])
            .args(["-D", &cfg.printer.virtual_info])
            .args(["-P", &format!("/{PPD_DEST}")])
            .arg("-E")
            .output()
            .context("cannot execute lpadmin")?;
        if !out.status.success() {
            bail!("lpadmin failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    })?;

    // 全局启用弹窗服务（对所有用户）
    plan.note("systemctl --global enable dupriman-prompt.service".to_string());
    run(opts, plan, || {
        let out = Command::new("systemctl")
            .args(["--global", "enable", "dupriman-prompt.service"])
            .output()
            .context("无法执行 systemctl")?;
        if !out.status.success() {
            // 非致命：可能某些环境不支持 --global
            eprintln!(
                "Warning: systemctl --global enable failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    })?;
    Ok(())
}

pub fn uninstall(cfg: &Config, opts: &InstallOpts, purge: bool) -> Result<()> {
    require_root(opts)?;
    let mut plan = Plan::new();

    if !opts.no_activate {
        plan.note(format!("Removing CUPS printer {}", cfg.printer.virtual_name));
        run(opts, &mut plan, || {
            let out = Command::new("lpadmin")
                .args(["-x", &cfg.printer.virtual_name])
                .output()
                .context("无法执行 lpadmin")?;
            if !out.status.success() {
                eprintln!(
                    "Warning: lpadmin -x failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Ok(())
        })?;
        plan.note("systemctl --global disable dupriman-prompt.service".to_string());
        run(opts, &mut plan, || {
            Command::new("systemctl")
                .args(["--global", "disable", "dupriman-prompt.service"])
                .output()
                .ok();
            Ok(())
        })?;
    }

    for rel in [BACKEND_DEST, PPD_DEST, UNIT_DEST, TMPFILES_DEST] {
        let p = under(&opts.root, rel);
        plan.note(format!("Removing {}", p.display()));
        run(opts, &mut plan, || {
            fs::remove_file(&p).ok();
            Ok(())
        })?;
    }
    let i18n_dir = under(&opts.root, I18N_DIR);
    plan.note(format!("Removing {}", i18n_dir.display()));
    run(opts, &mut plan, || {
        fs::remove_dir_all(&i18n_dir).ok();
        Ok(())
    })?;
    if purge {
        for rel in [BIN_DEST, CONFIG_DEST] {
            let p = under(&opts.root, rel);
            plan.note(format!("Removing {}", p.display()));
            run(opts, &mut plan, || {
                fs::remove_file(&p).ok();
                Ok(())
            })?;
        }
    } else {
        plan.note(format!("Keeping /{BIN_DEST} and /{CONFIG_DEST} (use --purge to remove)"));
    }

    println!("{}", cfg.prompt.cli("cli_uninstall_plan"));
    for a in &plan.actions {
        println!("  - {a}");
    }
    Ok(())
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn run(opts: &InstallOpts, plan: &mut Plan, f: impl FnOnce() -> Result<()>) -> Result<()> {
    if opts.dry_run {
        plan.note("  （dry-run，未执行）");
        return Ok(());
    }
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_files_under_root() {
        let dir = std::env::temp_dir().join(format!("dupriman-install-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let cfg = Config::default();
        let opts = InstallOpts {
            root: dir.clone(),
            no_activate: true,
            ..Default::default()
        };
        install(&cfg, &opts).unwrap();

        assert!(dir.join(BIN_DEST).exists());
        assert!(dir.join(BACKEND_DEST).symlink_metadata().is_ok());
        assert!(dir.join(PPD_DEST).exists());
        assert!(dir.join(CONFIG_DEST).exists());
        assert!(dir.join(UNIT_DEST).exists());
        assert!(dir.join(TMPFILES_DEST).exists());

        let tmpf = fs::read_to_string(dir.join(TMPFILES_DEST)).unwrap();
        assert!(tmpf.contains("/run/dupriman 1777"));

        fs::remove_dir_all(&dir).ok();
    }
}
