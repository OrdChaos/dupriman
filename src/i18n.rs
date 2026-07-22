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
    for lang in [Lang::En, Lang::ZhCn, Lang::ZhTw] {
        let stem = lang.file_stem();
        let mut texts = load_file(stem).unwrap_or_else(|| en.clone());
        // Fill any missing keys from en_US fallback
        let en = map.get("en_US").cloned().unwrap_or_else(|| en.clone());
        for key in [
            "title", "message", "ok_label", "cancel_label", "detail", "detail_with_blank",
            "cli_job_completed", "cli_waiting_confirmation", "cli_user_confirmed",
            "cli_printing_fronts", "cli_printing_backs", "cli_printing_backs_blank",
            "cli_saved_test", "cli_submitted", "cli_prompt_disabled", "cli_single_page",
            "cli_daemon_started", "cli_confirmation_request", "cli_failed_write_response",
            "cli_no_display", "cli_install_plan", "cli_uninstall_plan", "cli_done_confirm",
            "cli_all_passed", "cli_checks_failed",
            "ppd_page_size", "ppd_page_size_a4", "ppd_page_size_letter", "ppd_page_size_legal",
            "ppd_duplex", "ppd_duplex_none", "ppd_duplex_long_edge", "ppd_duplex_short_edge",
        ] {
            if texts.get(key).is_none() {
                if let Some(v) = en.get(key) {
                    set(&mut texts, key, v);
                }
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../share/dupriman/i18n").join(&name));
            candidates.push(dir.join("../../i18n").join(&name));
        }
    }
    candidates.push(PathBuf::from("./i18n").join(&name));

    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(t) = toml::from_str(&text) {
                return Some(t);
            }
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
