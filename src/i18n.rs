//! Internationalization: load user-visible text from `i18n/<locale>.toml` files.
//!
//! Resolution order for each text key:
//! 1. Custom value in dupriman config (`[prompt].title` etc.) — highest priority
//! 2. `i18n/<locale>.toml` matching the resolved language
//! 3. Built-in `en_US` — ultimate fallback
//!
//! Locale files are searched in:
//! - `/usr/share/dupriman/i18n/` (system-wide, installed)
//! - `<exe_dir>/../share/dupriman/i18n/` (development)
//! - `./i18n/` (repository root, cargo run/test)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[serde(rename = "en", alias = "en_US")]
    En,
    #[serde(rename = "zh_CN")]
    ZhCn,
    #[serde(rename = "zh_TW")]
    ZhTw,
}

impl Lang {
    /// All supported languages (keep in sync with the enum variants)
    pub const ALL: &[Lang] = &[Lang::En, Lang::ZhCn, Lang::ZhTw];

    /// Detect language from environment (`LC_ALL` / `LANG`)
    pub fn from_env() -> Self {
        let env = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default()
            .to_lowercase();
        if env.contains("zh_tw") || env.contains("zh_hk") {
            Self::ZhTw
        } else if env.contains("zh_cn") || env.contains("zh_sg") {
            Self::ZhCn
        } else {
            Self::En
        }
    }

    /// Locale file stem (e.g. `en_US`)
    pub fn file_stem(self) -> &'static str {
        match self {
            Self::En => "en_US",
            Self::ZhCn => "zh_CN",
            Self::ZhTw => "zh_TW",
        }
    }
}

/// All translatable text keys
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Texts {
    pub title: String,
    pub message: String,
    pub ok_label: String,
    pub cancel_label: String,
    pub detail: String,
    pub detail_with_blank: String,
    pub cli_job_completed: String,
    pub cli_waiting_confirmation: String,
    pub cli_user_confirmed: String,
    pub cli_printing_fronts: String,
    pub cli_printing_backs: String,
    pub cli_printing_backs_blank: String,
    pub cli_saved_test: String,
    pub cli_submitted: String,
    pub cli_prompt_disabled: String,
    pub cli_single_page: String,
    pub cli_daemon_started: String,
    pub cli_confirmation_request: String,
    pub cli_failed_write_response: String,
    pub cli_no_display: String,
    pub cli_install_plan: String,
    pub cli_uninstall_plan: String,
    pub cli_done_confirm: String,
    pub cli_all_passed: String,
    pub cli_checks_failed: String,
    pub ppd_page_size: String,
    pub ppd_page_size_a4: String,
    pub ppd_page_size_letter: String,
    pub ppd_page_size_legal: String,
    pub ppd_duplex: String,
    pub ppd_duplex_none: String,
    pub ppd_duplex_long_edge: String,
    pub ppd_duplex_short_edge: String,
    pub cli_args_error: String,
    pub cli_prompt_failed: String,
    pub cli_install_failed: String,
    pub cli_uninstall_failed: String,
    pub cli_config_warning: String,
    pub cli_cups_discovery_note: String,
    pub cli_dry_run_note: String,
    pub test_dep_header: String,
    pub test_dep_hint: String,
    pub test_config_header: String,
    pub test_config_label: String,
    pub test_config_hint: String,
    pub test_printers_header: String,
    pub test_real_printer_fmt: String,
    pub test_real_printer_hint: String,
    pub test_virtual_printer_fmt: String,
    pub test_virtual_printer_hint: String,
    pub test_runtime_header: String,
    pub test_state_dir_fmt: String,
    pub test_state_dir_hint: String,
    pub test_prompt_label: String,
    pub test_prompt_hint: String,
    pub install_root_required: String,
    pub install_binary: String,
    pub install_backend_symlink: String,
    pub install_ppd: String,
    pub install_config_skip: String,
    pub install_config_write: String,
    pub install_service: String,
    pub install_tmpfiles: String,
    pub install_i18n: String,
    pub install_no_activate: String,
    pub install_dirs: String,
    pub install_lpadmin: String,
    pub install_systemctl_enable: String,
    pub install_systemctl_warn: String,
    pub uninstall_removing: String,
    pub uninstall_lpadmin: String,
    pub uninstall_lpadmin_warn: String,
    pub uninstall_systemctl_disable: String,
    pub uninstall_keeping: String,
    pub print_lp_failed: String,
    pub print_lp_submit_failed: String,
    pub print_lp_parse_failed: String,
    pub print_lpstat_failed: String,
    pub print_cancel_failed: String,
    pub print_cancel_err: String,
    pub backend_stdin_read_failed: String,
    pub backend_user_cancelled: String,
    pub backend_timeout: String,
    pub backend_print_timeout: String,
    pub duplex_simplex: String,
    pub duplex_long_edge: String,
    pub duplex_short_edge: String,
}

impl Texts {
    /// Get a text by key name (returns None if missing or empty)
    pub fn get(&self, key: &str) -> Option<&str> {
        let s: &str = match key {
            "title" => &self.title,
            "message" => &self.message,
            "ok_label" => &self.ok_label,
            "cancel_label" => &self.cancel_label,
            "detail" => &self.detail,
            "detail_with_blank" => &self.detail_with_blank,
            "cli_job_completed" => &self.cli_job_completed,
            "cli_waiting_confirmation" => &self.cli_waiting_confirmation,
            "cli_user_confirmed" => &self.cli_user_confirmed,
            "cli_printing_fronts" => &self.cli_printing_fronts,
            "cli_printing_backs" => &self.cli_printing_backs,
            "cli_printing_backs_blank" => &self.cli_printing_backs_blank,
            "cli_saved_test" => &self.cli_saved_test,
            "cli_submitted" => &self.cli_submitted,
            "cli_prompt_disabled" => &self.cli_prompt_disabled,
            "cli_single_page" => &self.cli_single_page,
            "cli_daemon_started" => &self.cli_daemon_started,
            "cli_confirmation_request" => &self.cli_confirmation_request,
            "cli_failed_write_response" => &self.cli_failed_write_response,
            "cli_no_display" => &self.cli_no_display,
            "cli_install_plan" => &self.cli_install_plan,
            "cli_uninstall_plan" => &self.cli_uninstall_plan,
            "cli_done_confirm" => &self.cli_done_confirm,
            "cli_all_passed" => &self.cli_all_passed,
            "cli_checks_failed" => &self.cli_checks_failed,
            "ppd_page_size" => &self.ppd_page_size,
            "ppd_page_size_a4" => &self.ppd_page_size_a4,
            "ppd_page_size_letter" => &self.ppd_page_size_letter,
            "ppd_page_size_legal" => &self.ppd_page_size_legal,
            "ppd_duplex" => &self.ppd_duplex,
            "ppd_duplex_none" => &self.ppd_duplex_none,
            "ppd_duplex_long_edge" => &self.ppd_duplex_long_edge,
            "ppd_duplex_short_edge" => &self.ppd_duplex_short_edge,
            "cli_args_error" => &self.cli_args_error,
            "cli_prompt_failed" => &self.cli_prompt_failed,
            "cli_install_failed" => &self.cli_install_failed,
            "cli_uninstall_failed" => &self.cli_uninstall_failed,
            "cli_config_warning" => &self.cli_config_warning,
            "cli_cups_discovery_note" => &self.cli_cups_discovery_note,
            "cli_dry_run_note" => &self.cli_dry_run_note,
            "test_dep_header" => &self.test_dep_header,
            "test_dep_hint" => &self.test_dep_hint,
            "test_config_header" => &self.test_config_header,
            "test_config_label" => &self.test_config_label,
            "test_config_hint" => &self.test_config_hint,
            "test_printers_header" => &self.test_printers_header,
            "test_real_printer_fmt" => &self.test_real_printer_fmt,
            "test_real_printer_hint" => &self.test_real_printer_hint,
            "test_virtual_printer_fmt" => &self.test_virtual_printer_fmt,
            "test_virtual_printer_hint" => &self.test_virtual_printer_hint,
            "test_runtime_header" => &self.test_runtime_header,
            "test_state_dir_fmt" => &self.test_state_dir_fmt,
            "test_state_dir_hint" => &self.test_state_dir_hint,
            "test_prompt_label" => &self.test_prompt_label,
            "test_prompt_hint" => &self.test_prompt_hint,
            "install_root_required" => &self.install_root_required,
            "install_binary" => &self.install_binary,
            "install_backend_symlink" => &self.install_backend_symlink,
            "install_ppd" => &self.install_ppd,
            "install_config_skip" => &self.install_config_skip,
            "install_config_write" => &self.install_config_write,
            "install_service" => &self.install_service,
            "install_tmpfiles" => &self.install_tmpfiles,
            "install_i18n" => &self.install_i18n,
            "install_no_activate" => &self.install_no_activate,
            "install_dirs" => &self.install_dirs,
            "install_lpadmin" => &self.install_lpadmin,
            "install_systemctl_enable" => &self.install_systemctl_enable,
            "install_systemctl_warn" => &self.install_systemctl_warn,
            "uninstall_removing" => &self.uninstall_removing,
            "uninstall_lpadmin" => &self.uninstall_lpadmin,
            "uninstall_lpadmin_warn" => &self.uninstall_lpadmin_warn,
            "uninstall_systemctl_disable" => &self.uninstall_systemctl_disable,
            "uninstall_keeping" => &self.uninstall_keeping,
            "print_lp_failed" => &self.print_lp_failed,
            "print_lp_submit_failed" => &self.print_lp_submit_failed,
            "print_lp_parse_failed" => &self.print_lp_parse_failed,
            "print_lpstat_failed" => &self.print_lpstat_failed,
            "print_cancel_failed" => &self.print_cancel_failed,
            "print_cancel_err" => &self.print_cancel_err,
            "backend_stdin_read_failed" => &self.backend_stdin_read_failed,
            "backend_user_cancelled" => &self.backend_user_cancelled,
            "backend_timeout" => &self.backend_timeout,
            "backend_print_timeout" => &self.backend_print_timeout,
            "duplex_simplex" => &self.duplex_simplex,
            "duplex_long_edge" => &self.duplex_long_edge,
            "duplex_short_edge" => &self.duplex_short_edge,
            _ => return None,
        };
        if s.is_empty() { None } else { Some(s) }
    }
}

/// Built-in English fallback (compiled into the binary)
const EN_US: &str = include_str!("../i18n/en_US.toml");

static CACHE: OnceLock<HashMap<&'static str, Texts>> = OnceLock::new();

/// Load translations for all languages, with fallback chain:
/// locale file -> en_US file -> built-in en_US
fn load_all() -> HashMap<&'static str, Texts> {
    let en: Texts = toml::from_str(EN_US).unwrap_or_default();
    let mut map = HashMap::new();
    for &lang in Lang::ALL {
        let stem = lang.file_stem();
        let mut texts = load_file(stem).unwrap_or_else(|| en.clone());
        // Fill any missing keys from en_US fallback
        let en = map.get("en_US").cloned().unwrap_or_else(|| en.clone());
        for key in [
            "title",
            "message",
            "ok_label",
            "cancel_label",
            "detail",
            "detail_with_blank",
            "cli_job_completed",
            "cli_waiting_confirmation",
            "cli_user_confirmed",
            "cli_printing_fronts",
            "cli_printing_backs",
            "cli_printing_backs_blank",
            "cli_saved_test",
            "cli_submitted",
            "cli_prompt_disabled",
            "cli_single_page",
            "cli_daemon_started",
            "cli_confirmation_request",
            "cli_failed_write_response",
            "cli_no_display",
            "cli_install_plan",
            "cli_uninstall_plan",
            "cli_done_confirm",
            "cli_all_passed",
            "cli_checks_failed",
            "ppd_page_size",
            "ppd_page_size_a4",
            "ppd_page_size_letter",
            "ppd_page_size_legal",
            "ppd_duplex",
            "ppd_duplex_none",
            "ppd_duplex_long_edge",
            "ppd_duplex_short_edge",
            "cli_args_error",
            "cli_prompt_failed",
            "cli_install_failed",
            "cli_uninstall_failed",
            "cli_config_warning",
            "cli_cups_discovery_note",
            "cli_dry_run_note",
            "test_dep_header",
            "test_dep_hint",
            "test_config_header",
            "test_config_label",
            "test_config_hint",
            "test_printers_header",
            "test_real_printer_fmt",
            "test_real_printer_hint",
            "test_virtual_printer_fmt",
            "test_virtual_printer_hint",
            "test_runtime_header",
            "test_state_dir_fmt",
            "test_state_dir_hint",
            "test_prompt_label",
            "test_prompt_hint",
            "install_root_required",
            "install_binary",
            "install_backend_symlink",
            "install_ppd",
            "install_config_skip",
            "install_config_write",
            "install_service",
            "install_tmpfiles",
            "install_i18n",
            "install_no_activate",
            "install_dirs",
            "install_lpadmin",
            "install_systemctl_enable",
            "install_systemctl_warn",
            "uninstall_removing",
            "uninstall_lpadmin",
            "uninstall_lpadmin_warn",
            "uninstall_systemctl_disable",
            "uninstall_keeping",
            "print_lp_failed",
            "print_lp_submit_failed",
            "print_lp_parse_failed",
            "print_lpstat_failed",
            "print_cancel_failed",
            "print_cancel_err",
            "backend_stdin_read_failed",
            "backend_user_cancelled",
            "backend_timeout",
            "backend_print_timeout",
            "duplex_simplex",
            "duplex_long_edge",
            "duplex_short_edge",
        ] {
            if texts.get(key).is_none()
                && let Some(v) = en.get(key)
            {
                set(&mut texts, key, v);
            }
        }
        map.insert(stem, texts);
    }
    map
}

fn set(texts: &mut Texts, key: &str, value: &str) {
    let v = value.to_string();
    match key {
        "title" => texts.title = v,
        "message" => texts.message = v,
        "ok_label" => texts.ok_label = v,
        "cancel_label" => texts.cancel_label = v,
        "detail" => texts.detail = v,
        "detail_with_blank" => texts.detail_with_blank = v,
        "cli_job_completed" => texts.cli_job_completed = v,
        "cli_waiting_confirmation" => texts.cli_waiting_confirmation = v,
        "cli_user_confirmed" => texts.cli_user_confirmed = v,
        "cli_printing_fronts" => texts.cli_printing_fronts = v,
        "cli_printing_backs" => texts.cli_printing_backs = v,
        "cli_printing_backs_blank" => texts.cli_printing_backs_blank = v,
        "cli_saved_test" => texts.cli_saved_test = v,
        "cli_submitted" => texts.cli_submitted = v,
        "cli_prompt_disabled" => texts.cli_prompt_disabled = v,
        "cli_single_page" => texts.cli_single_page = v,
        "cli_daemon_started" => texts.cli_daemon_started = v,
        "cli_confirmation_request" => texts.cli_confirmation_request = v,
        "cli_failed_write_response" => texts.cli_failed_write_response = v,
        "cli_no_display" => texts.cli_no_display = v,
        "cli_install_plan" => texts.cli_install_plan = v,
        "cli_uninstall_plan" => texts.cli_uninstall_plan = v,
        "cli_done_confirm" => texts.cli_done_confirm = v,
        "cli_all_passed" => texts.cli_all_passed = v,
        "cli_checks_failed" => texts.cli_checks_failed = v,
        "ppd_page_size" => texts.ppd_page_size = v,
        "ppd_page_size_a4" => texts.ppd_page_size_a4 = v,
        "ppd_page_size_letter" => texts.ppd_page_size_letter = v,
        "ppd_page_size_legal" => texts.ppd_page_size_legal = v,
        "ppd_duplex" => texts.ppd_duplex = v,
        "ppd_duplex_none" => texts.ppd_duplex_none = v,
        "ppd_duplex_long_edge" => texts.ppd_duplex_long_edge = v,
        "ppd_duplex_short_edge" => texts.ppd_duplex_short_edge = v,
        "cli_args_error" => texts.cli_args_error = v,
        "cli_prompt_failed" => texts.cli_prompt_failed = v,
        "cli_install_failed" => texts.cli_install_failed = v,
        "cli_uninstall_failed" => texts.cli_uninstall_failed = v,
        "cli_config_warning" => texts.cli_config_warning = v,
        "cli_cups_discovery_note" => texts.cli_cups_discovery_note = v,
        "cli_dry_run_note" => texts.cli_dry_run_note = v,
        "test_dep_header" => texts.test_dep_header = v,
        "test_dep_hint" => texts.test_dep_hint = v,
        "test_config_header" => texts.test_config_header = v,
        "test_config_label" => texts.test_config_label = v,
        "test_config_hint" => texts.test_config_hint = v,
        "test_printers_header" => texts.test_printers_header = v,
        "test_real_printer_fmt" => texts.test_real_printer_fmt = v,
        "test_real_printer_hint" => texts.test_real_printer_hint = v,
        "test_virtual_printer_fmt" => texts.test_virtual_printer_fmt = v,
        "test_virtual_printer_hint" => texts.test_virtual_printer_hint = v,
        "test_runtime_header" => texts.test_runtime_header = v,
        "test_state_dir_fmt" => texts.test_state_dir_fmt = v,
        "test_state_dir_hint" => texts.test_state_dir_hint = v,
        "test_prompt_label" => texts.test_prompt_label = v,
        "test_prompt_hint" => texts.test_prompt_hint = v,
        "install_root_required" => texts.install_root_required = v,
        "install_binary" => texts.install_binary = v,
        "install_backend_symlink" => texts.install_backend_symlink = v,
        "install_ppd" => texts.install_ppd = v,
        "install_config_skip" => texts.install_config_skip = v,
        "install_config_write" => texts.install_config_write = v,
        "install_service" => texts.install_service = v,
        "install_tmpfiles" => texts.install_tmpfiles = v,
        "install_i18n" => texts.install_i18n = v,
        "install_no_activate" => texts.install_no_activate = v,
        "install_dirs" => texts.install_dirs = v,
        "install_lpadmin" => texts.install_lpadmin = v,
        "install_systemctl_enable" => texts.install_systemctl_enable = v,
        "install_systemctl_warn" => texts.install_systemctl_warn = v,
        "uninstall_removing" => texts.uninstall_removing = v,
        "uninstall_lpadmin" => texts.uninstall_lpadmin = v,
        "uninstall_lpadmin_warn" => texts.uninstall_lpadmin_warn = v,
        "uninstall_systemctl_disable" => texts.uninstall_systemctl_disable = v,
        "uninstall_keeping" => texts.uninstall_keeping = v,
        "print_lp_failed" => texts.print_lp_failed = v,
        "print_lp_submit_failed" => texts.print_lp_submit_failed = v,
        "print_lp_parse_failed" => texts.print_lp_parse_failed = v,
        "print_lpstat_failed" => texts.print_lpstat_failed = v,
        "print_cancel_failed" => texts.print_cancel_failed = v,
        "print_cancel_err" => texts.print_cancel_err = v,
        "backend_stdin_read_failed" => texts.backend_stdin_read_failed = v,
        "backend_user_cancelled" => texts.backend_user_cancelled = v,
        "backend_timeout" => texts.backend_timeout = v,
        "backend_print_timeout" => texts.backend_print_timeout = v,
        "duplex_simplex" => texts.duplex_simplex = v,
        "duplex_long_edge" => texts.duplex_long_edge = v,
        "duplex_short_edge" => texts.duplex_short_edge = v,
        _ => {}
    }
}

/// Try to load `i18n/<stem>.toml` from known locations
fn load_file(stem: &str) -> Option<Texts> {
    let name = format!("{stem}.toml");
    let mut candidates = vec![];
    // System-wide installed location
    candidates.push(PathBuf::from("/usr/share/dupriman/i18n").join(&name));
    // Development: <exe>/../../share/dupriman/i18n or ./i18n
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("../share/dupriman/i18n").join(&name));
        candidates.push(dir.join("../../i18n").join(&name));
    }
    candidates.push(PathBuf::from("./i18n").join(&name));

    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(t) = toml::from_str(&text)
        {
            return Some(t);
        }
    }
    None
}

/// Get translations for a language (with en_US fallback)
pub fn texts(lang: Lang) -> &'static Texts {
    let map = CACHE.get_or_init(load_all);
    map.get(lang.file_stem())
        .or_else(|| map.get("en_US"))
        .expect("en_US fallback must exist")
}

/// Get all loaded translations (for PPD generation etc.)
pub fn all_texts_map() -> &'static HashMap<&'static str, Texts> {
    CACHE.get_or_init(load_all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_fallback_always_works() {
        let t = texts(Lang::En);
        assert_eq!(t.title, "Duplex Printing");
        assert_eq!(t.ok_label, "OK");
        assert!(t.detail.contains("{job_id}"));
    }

    #[test]
    fn zh_cn_loaded() {
        let t = texts(Lang::ZhCn);
        assert_eq!(t.title, "双面打印");
        assert!(t.detail.contains("{job_id}"));
    }

    #[test]
    fn zh_tw_loaded() {
        let t = texts(Lang::ZhTw);
        assert_eq!(t.title, "雙面列印");
    }

    #[test]
    fn ppd_translations_loaded() {
        let t = texts(Lang::ZhCn);
        assert_eq!(t.ppd_duplex, "双面打印");
        assert_eq!(t.ppd_duplex_none, "单面");
        assert_eq!(t.ppd_duplex_long_edge, "长边翻转");
        assert_eq!(t.ppd_duplex_short_edge, "短边翻转");
        assert_eq!(t.ppd_page_size, "纸张大小");
    }

    #[test]
    fn all_texts_map_has_all_langs() {
        let map = all_texts_map();
        assert!(map.contains_key("en_US"));
        assert!(map.contains_key("zh_CN"));
        assert!(map.contains_key("zh_TW"));
    }

    #[test]
    fn detail_format() {
        let t = texts(Lang::En);
        let s = t
            .detail
            .replace("{job_id}", "42")
            .replace("{title}", "doc")
            .replace("{total}", "10")
            .replace("{evens}", "5");
        assert_eq!(
            s,
            "Job #42 \"doc\" (10 pages): all fronts printed, 5 back pages pending."
        );
    }
}
