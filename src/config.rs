//! 配置模块：TOML 配置文件，系统级 + 用户级覆盖合并。
//!
//! 加载顺序（后者覆盖前者）：
//! 1. 内置默认值
//! 2. `/etc/dupriman/config.toml`（系统级）
//! 3. `~/.config/dupriman/config.toml`（用户级，`$XDG_CONFIG_HOME` 优先）
//! 4. `--config` 显式指定的文件（如有）

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::i18n::Lang;
use crate::options::DuplexMode;

/// 系统级配置文件路径
pub const SYSTEM_CONFIG: &str = "/etc/dupriman/config.toml";

/// 顶层配置
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub printer: PrinterCfg,
    pub paths: PathsCfg,
    pub duplex: DuplexCfg,
    pub prompt: PromptCfg,
    pub print: PrintCfg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrinterCfg {
    /// 真实打印机 CUPS 队列名（单面物理打印机）
    pub real: String,
    /// 要注册的虚拟打印机名
    pub virtual_name: String,
    /// 虚拟打印机设备 URI（backend 入口）
    pub virtual_uri: String,
    /// 虚拟打印机在 CUPS 中显示的描述
    pub virtual_info: String,
}

impl Default for PrinterCfg {
    fn default() -> Self {
        Self {
            // 默认输出到 PDF（测试模式），避免误消耗纸张；
            // 真机打印时改为实际队列名（如 HP_LaserJet_1020）
            real: "save-pdf".into(),
            virtual_name: "dupriman-duplex".into(),
            virtual_uri: "dupriman://duplex".into(),
            virtual_info: "DupriMan Manual Duplex".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsCfg {
    /// 后端与弹窗守护进程通信的根目录（root-owned, non-writable）
    pub state_dir: PathBuf,
    /// 拆分出的临时 PDF 存放目录
    pub spool_dir: PathBuf,
}

impl Default for PathsCfg {
    fn default() -> Self {
        Self {
            state_dir: PathBuf::from("/run/dupriman"),
            spool_dir: PathBuf::from("/var/tmp/dupriman"),
        }
    }
}

/// 偶数页顺序
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvenOrder {
    Normal,
    Reverse,
}

/// 某一翻面方式（长边/短边）的偶数页参数
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SideCfg {
    /// 偶数页打印顺序；None = 继承 `[duplex].even_order`
    pub even_order: Option<EvenOrder>,
    /// 偶数页整体旋转角度（0/90/180/270），短边翻转通常需要 180
    pub even_rotate: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplexCfg {
    /// 偶数页默认顺序（长边/短边可用 even_order 单独覆盖）
    pub even_order: EvenOrder,
    pub long_edge: SideCfg,
    pub short_edge: SideCfg,
}

impl Default for DuplexCfg {
    fn default() -> Self {
        // 出纸面朝下的打印机（如 HP LaserJet 1020），用户"原样不动"地把
        // 纸叠放回进纸口。此时纸叠顶→底 = 先打→后打（正面先打的在底），
        // 进纸从顶部开始，所以背面必须**倒序**（先打最后一页，落在最上面的纸上）。
        // 旋转角度与翻纸方向有关：
        // - 长边翻转：原样放回后背面上下与正面同向 -> 需旋转 180°
        // - 短边翻转：纸被调转方向，原样放回后背面上下与正面相反 -> 不旋转
        Self {
            even_order: EvenOrder::Reverse,
            long_edge: SideCfg {
                even_order: None,
                even_rotate: 180,
            },
            short_edge: SideCfg {
                even_order: None,
                even_rotate: 0,
            },
        }
    }
}

impl DuplexCfg {
    /// 解析某翻面方式最终生效的 (偶数页顺序, 旋转角度)
    pub fn for_mode(&self, mode: DuplexMode) -> (EvenOrder, i32) {
        let side = match mode {
            DuplexMode::ShortEdge => &self.short_edge,
            _ => &self.long_edge,
        };
        (side.even_order.unwrap_or(self.even_order), side.even_rotate)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptCfg {
    /// 是否启用弹窗确认；false 时打完奇数页立刻打偶数页（用于调试）
    pub enabled: bool,
    /// 等待用户点击的秒数，0 = 无限等待；超时按取消处理
    pub timeout_secs: u64,
    /// 弹窗语言：auto（跟随环境）| en | zh_CN | zh_TW
    pub language: String,
    /// 弹窗标题（留空则按 language 自动选择）
    pub title: String,
    /// 弹窗提示文案（留空则按 language 自动选择）
    pub message: String,
    /// 确认按钮文字（留空则按 language 自动选择）
    pub ok_label: String,
    /// 取消按钮文字（留空则按 language 自动选择）
    pub cancel_label: String,
    /// 可接收确认请求的本地账户。留空时拒绝双面作业。
    pub allowed_users: Vec<String>,
}

impl PromptCfg {
    /// 解析最终生效的语言
    pub fn lang(&self) -> Lang {
        match self.language.to_lowercase().as_str() {
            "zh_cn" => Lang::ZhCn,
            "zh_tw" | "zh_hk" => Lang::ZhTw,
            "en" | "en_us" => Lang::En,
            _ => Lang::from_env(),
        }
    }

    /// 当前语言的完整翻译文本（含 en_US 兜底）
    fn texts(&self) -> &'static crate::i18n::Texts {
        crate::i18n::texts(self.lang())
    }

    /// 弹窗标题（配置优先，否则按语言）
    pub fn resolved_title(&self) -> String {
        if self.title.is_empty() {
            self.texts().title.clone()
        } else {
            self.title.clone()
        }
    }

    /// 弹窗提示文案（配置优先，否则按语言）
    pub fn resolved_message(&self) -> String {
        if self.message.is_empty() {
            self.texts().message.clone()
        } else {
            self.message.clone()
        }
    }

    /// 确认按钮文字（配置优先，否则按语言）
    pub fn resolved_ok_label(&self) -> String {
        if self.ok_label.is_empty() {
            self.texts().ok_label.clone()
        } else {
            self.ok_label.clone()
        }
    }

    /// 取消按钮文字（配置优先，否则按语言）
    pub fn resolved_cancel_label(&self) -> String {
        if self.cancel_label.is_empty() {
            self.texts().cancel_label.clone()
        } else {
            self.cancel_label.clone()
        }
    }

    /// 任务详情文案（配置优先，否则按语言；含占位符替换）
    pub fn resolved_detail(
        &self,
        job_id: &str,
        title: &str,
        total: u32,
        evens: usize,
        has_blank: bool,
    ) -> String {
        let template = if has_blank {
            &self.texts().detail_with_blank
        } else {
            &self.texts().detail
        };
        template
            .replace("{job_id}", job_id)
            .replace("{title}", title)
            .replace("{total}", &total.to_string())
            .replace("{evens}", &evens.to_string())
    }

    /// 获取 CLI 提示文本（按语言，支持占位符替换）
    pub fn cli(&self, key: &str) -> String {
        self.texts().get(key).unwrap_or(key).to_string()
    }

    /// CLI 文本占位符替换
    pub fn cli_fmt(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.cli(key);
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

impl Default for PromptCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 600,
            language: "auto".into(),
            title: String::new(),
            message: String::new(),
            ok_label: String::new(),
            cancel_label: String::new(),
            allowed_users: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintCfg {
    /// 轮询打印作业状态的间隔（毫秒）
    pub poll_interval_ms: u64,
    /// 等待单个打印作业完成的超时（秒）
    pub job_timeout_secs: u64,
    /// 转发给真实打印机的选项白名单（如 media、fit-to-page）
    pub forward_options: Vec<String>,
    /// 保留临时文件用于调试
    pub keep_temp: bool,
}

impl Default for PrintCfg {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            job_timeout_secs: 3600,
            forward_options: vec!["media".into(), "fit-to-page".into()],
            keep_temp: false,
        }
    }
}

impl Config {
    /// 按优先级加载并合并配置
    pub fn load(extra: Option<&Path>) -> Result<Self> {
        Self::load_paths(Self::candidate_paths(extra))
    }

    /// Load configuration used by the CUPS backend.  A backend must never
    /// consume an arbitrary user's configuration while making privileged
    /// printing decisions.
    pub fn load_for_backend(extra: Option<&Path>) -> Result<Self> {
        let mut paths = vec![PathBuf::from(SYSTEM_CONFIG)];
        if let Some(path) = extra {
            paths.push(path.to_path_buf());
        }
        Self::load_paths(paths)
    }

    fn load_paths(paths: Vec<PathBuf>) -> Result<Self> {
        let mut base = toml::Value::Table(toml::map::Map::new());
        for path in paths {
            if !path.exists() {
                continue;
            }
            let text = fs::read_to_string(&path)
                .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
            let value: toml::Value = toml::from_str(&text)
                .with_context(|| format!("解析配置文件失败: {}", path.display()))?;
            merge_value(&mut base, value);
        }
        let cfg: Config = base
            .try_into()
            .context("配置字段类型错误，请检查配置文件")?;
        Ok(cfg)
    }

    /// 候选配置文件路径（按加载优先级排序）
    pub fn candidate_paths(extra: Option<&Path>) -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from(SYSTEM_CONFIG)];
        if let Some(dir) = user_config_dir() {
            paths.push(dir.join("dupriman").join("config.toml"));
        }
        if let Some(p) = extra {
            paths.push(p.to_path_buf());
        }
        paths
    }

    /// 生成一份带完整注释的示例配置（供 install 写入 /etc）
    pub fn sample_toml() -> String {
        let cfg = Config::default();
        format!(
            r#"# DupriMan configuration file
# User-level override: ~/.config/dupriman/config.toml (only write fields to override)

[printer]
# Real printer CUPS queue name (`lpstat -v` to list)
# Special value "save-pdf" saves jobs to spool_dir/saved/ instead of printing (test mode)
real = "{real}"
# Virtual printer name to register (select this in print dialogs)
virtual_name = "{vname}"
virtual_uri = "{vuri}"
virtual_info = "{vinfo}"

[paths]
# Root directory for backend <-> prompt daemon communication.  It must be
# root-owned and not writable by untrusted users.  Production integration
# provisions a root-owned per-user directory with requests/ and responses/
# children using the dedicated dupriman-<user> bridge group.
state_dir = "{state}"
# Temporary directory for split PDFs
spool_dir = "{spool}"

[duplex]
# Default even-page order: reverse (face-down output trays like HP LaserJet) | normal
even_order = "{order}"

[duplex.long_edge]
# Long-edge flip: rotation angle for even pages
even_rotate = {lr}

[duplex.short_edge]
# Short-edge flip: rotation angle for even pages
even_rotate = {sr}
# even_order = "normal"   # Uncomment to override order for this mode only

[prompt]
enabled = {penabled}
# Language for dialog: auto (detect from environment) | en | zh_CN | zh_TW
language = "{plang}"
# Seconds to wait for user click, 0 = wait forever
timeout_secs = {ptimeout}
# Custom dialog text (leave empty to use language defaults)
title = "{ptitle}"
message = "{pmsg}"
ok_label = "{pok}"
cancel_label = "{pcancel}"
# Local accounts allowed to receive duplex confirmation prompts.  Keep this
# explicit; an empty list rejects duplex jobs.
allowed_users = {allowed_users:?}

[print]
poll_interval_ms = {poll}
job_timeout_secs = {jtimeout}
# Options forwarded to the real printer (whitelist)
forward_options = {fwd:?}
keep_temp = {keep}
"#,
            real = cfg.printer.real,
            vname = cfg.printer.virtual_name,
            vuri = cfg.printer.virtual_uri,
            vinfo = cfg.printer.virtual_info,
            state = cfg.paths.state_dir.display(),
            spool = cfg.paths.spool_dir.display(),
            order = match cfg.duplex.even_order {
                EvenOrder::Reverse => "reverse",
                EvenOrder::Normal => "normal",
            },
            lr = cfg.duplex.long_edge.even_rotate,
            sr = cfg.duplex.short_edge.even_rotate,
            penabled = cfg.prompt.enabled,
            plang = cfg.prompt.language,
            ptimeout = cfg.prompt.timeout_secs,
            ptitle = cfg.prompt.title,
            pmsg = cfg.prompt.message,
            pok = cfg.prompt.ok_label,
            pcancel = cfg.prompt.cancel_label,
            allowed_users = cfg.prompt.allowed_users,
            poll = cfg.print.poll_interval_ms,
            jtimeout = cfg.print.job_timeout_secs,
            fwd = cfg.print.forward_options,
            keep = cfg.print.keep_temp,
        )
    }
}

/// 用户配置目录（$XDG_CONFIG_HOME 或 ~/.config）
fn user_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".config"))
}

/// 深合并两个 TOML 值：表递归合并，其余直接覆盖
fn merge_value(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) => merge_value(bv, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let cfg = Config::default();
        // 默认输出到 PDF（测试模式），避免误消耗纸张
        assert_eq!(cfg.printer.real, "save-pdf");
        // 默认语言 auto，文案留空由 i18n 按语言填充
        assert_eq!(cfg.prompt.language, "auto");
        assert!(cfg.prompt.message.is_empty());
        assert!(cfg.prompt.allowed_users.is_empty());
        // 出纸面朝下 + 原样放回纸叠：纸叠顶→底 = 先打→后打，进纸从顶开始，
        // 故背面倒序（Reverse）；长边翻转背面转 180°，短边翻转不转。
        let (order, rotate) = cfg.duplex.for_mode(DuplexMode::LongEdge);
        assert_eq!(rotate, 180);
        assert_eq!(order, EvenOrder::Reverse);
        let (order, rotate) = cfg.duplex.for_mode(DuplexMode::ShortEdge);
        assert_eq!(rotate, 0);
        assert_eq!(order, EvenOrder::Reverse);
    }

    #[test]
    fn prompt_language_resolution() {
        // auto 模式跟随环境，测试环境可能是中文，所以只验证非 auto 的显式设置
        let mut cfg = PromptCfg {
            language: "en".into(),
            ..Default::default()
        };
        assert_eq!(cfg.lang(), crate::i18n::Lang::En);
        assert_eq!(cfg.resolved_title(), "Duplex Printing");
        assert_eq!(cfg.resolved_ok_label(), "OK");

        cfg.language = "zh_CN".into();
        assert_eq!(cfg.lang(), crate::i18n::Lang::ZhCn);
        assert_eq!(cfg.resolved_title(), "双面打印");
        assert_eq!(cfg.resolved_cancel_label(), "取消");

        cfg.language = "zh_TW".into();
        assert_eq!(cfg.lang(), crate::i18n::Lang::ZhTw);
        assert_eq!(cfg.resolved_title(), "雙面列印");

        // 自定义文案优先
        cfg.message = "Custom message".into();
        assert_eq!(cfg.resolved_message(), "Custom message");
    }

    #[test]
    fn merge_overrides_only_given_keys() {
        let mut base = toml::Value::Table(toml::map::Map::new());
        merge_value(
            &mut base,
            toml::from_str(
                r#"[prompt]
message = "系统文案"
"#,
            )
            .unwrap(),
        );
        merge_value(
            &mut base,
            toml::from_str(
                r#"[prompt]
ok_label = "好"
"#,
            )
            .unwrap(),
        );
        let cfg: Config = base.try_into().unwrap();
        assert_eq!(cfg.prompt.message, "系统文案");
        assert_eq!(cfg.prompt.ok_label, "好");
        // 未指定的字段保持默认
        assert_eq!(cfg.prompt.cancel_label, "");
    }

    #[test]
    fn sample_toml_parses_to_defaults() {
        let text = Config::sample_toml();
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, Config::default());
    }
}
