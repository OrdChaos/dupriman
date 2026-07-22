//! CUPS 打印选项解析：从后端 argv 的 options 字符串中识别双面模式。

/// 双面模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplexMode {
    /// 单面
    Simplex,
    /// 双面 - 长边翻转
    LongEdge,
    /// 双面 - 短边翻转
    ShortEdge,
}

impl std::fmt::Display for DuplexMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DuplexMode::Simplex => write!(f, "单面"),
            DuplexMode::LongEdge => write!(f, "双面-长边翻转"),
            DuplexMode::ShortEdge => write!(f, "双面-短边翻转"),
        }
    }
}

/// 解析后的作业选项
#[derive(Debug, Clone)]
pub struct JobOptions {
    pub mode: DuplexMode,
    /// 全部原始键值对（无值布尔键记为 "true"）
    pub pairs: Vec<(String, String)>,
}

impl JobOptions {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// 解析 CUPS options 字符串，形如：
/// `Duplex=DuplexNoTumble media=A4 number-up=1 fit-to-page`
pub fn parse(options: &str) -> JobOptions {
    let mut pairs = Vec::new();
    for token in options.split_whitespace() {
        match token.split_once('=') {
            Some((k, v)) => pairs.push((k.to_string(), v.to_string())),
            None => pairs.push((token.to_string(), "true".to_string())),
        }
    }
    let mode = detect_mode(&pairs);
    JobOptions { mode, pairs }
}

fn detect_mode(pairs: &[(String, String)]) -> DuplexMode {
    // 依次检查 Duplex 与 sides 两个常见键
    for key in ["Duplex", "sides"] {
        if let Some((_, value)) = pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            let v = value.to_ascii_lowercase();
            // 注意先判断 notumble，因为 "duplextumble" 也包含 "tumble"
            if v.contains("notumble") || v.contains("long-edge") {
                return DuplexMode::LongEdge;
            }
            if v.contains("tumble") || v.contains("short-edge") {
                return DuplexMode::ShortEdge;
            }
            // 显式设置了但非双面值（None / one-sided 等）
            return DuplexMode::Simplex;
        }
    }
    DuplexMode::Simplex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duplex_long_edge() {
        assert_eq!(parse("Duplex=DuplexNoTumble").mode, DuplexMode::LongEdge);
        assert_eq!(
            parse("sides=two-sided-long-edge media=A4").mode,
            DuplexMode::LongEdge
        );
    }

    #[test]
    fn parses_duplex_short_edge() {
        assert_eq!(parse("Duplex=DuplexTumble").mode, DuplexMode::ShortEdge);
        assert_eq!(parse("sides=two-sided-short-edge").mode, DuplexMode::ShortEdge);
    }

    #[test]
    fn parses_simplex() {
        assert_eq!(parse("").mode, DuplexMode::Simplex);
        assert_eq!(parse("Duplex=None").mode, DuplexMode::Simplex);
        assert_eq!(parse("sides=one-sided").mode, DuplexMode::Simplex);
        assert_eq!(parse("media=A4 fit-to-page").mode, DuplexMode::Simplex);
    }

    #[test]
    fn boolean_keys_become_true() {
        let opts = parse("fit-to-page media=A4");
        assert_eq!(opts.get("fit-to-page"), Some("true"));
        assert_eq!(opts.get("media"), Some("A4"));
        assert_eq!(opts.get("missing"), None);
    }
}
