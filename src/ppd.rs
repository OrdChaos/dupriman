//! 虚拟打印机 PPD 生成。
//!
//! 要点：
//! - `cupsFilter application/vnd.cups-pdf 0 -`：声明队列直接接受 PDF，
//!   CUPS 不再插入其他过滤器，后端收到的就是 PDF。
//! - `cupsManualCopies: True`：多份拷贝交给后端处理（正/背面各提交一次
//!   带份数的子作业，避免过滤器复制页面导致正背面交错）。
//! - `Duplex` UI 选项：单面 / 长边翻转 / 短边翻转，原样传给后端 argv。
//! - PPD 翻译文本来自 i18n 系统，无需硬编码。

use crate::config::Config;
use crate::i18n;

pub fn generate(cfg: &Config) -> String {
    let name = &cfg.printer.virtual_name;
    let info = &cfg.printer.virtual_info;

    // 收集 cupsLanguages 和翻译行
    let all = i18n::all_texts_map();
    let mut cups_langs: Vec<&str> = Vec::new();
    let mut translation_lines = String::new();

    for (&stem, texts) in all.iter() {
        if stem == "en_US" {
            cups_langs.push("en");
            continue; // 英文是基座语言，不需要 Translation 行
        }
        // PPD 使用 locale 名本身作为语言标签（如 zh_CN、zh_TW）
        let ppd_lang = stem;
        cups_langs.push(ppd_lang);

        // 选项组翻译
        translation_lines.push_str(&format!(
            "*{lang}.Translation PageSize/{text}: \"\"\n",
            lang = ppd_lang,
            text = texts.ppd_page_size
        ));
        translation_lines.push_str(&format!(
            "*{lang}.Translation PageRegion/{text}: \"\"\n",
            lang = ppd_lang,
            text = texts.ppd_page_size
        ));
        translation_lines.push_str(&format!(
            "*{lang}.Translation Duplex/{text}: \"\"\n",
            lang = ppd_lang,
            text = texts.ppd_duplex
        ));

        // 选项值翻译
        for (opt, value, text) in [
            ("PageSize", "A4", &texts.ppd_page_size_a4),
            ("PageSize", "Letter", &texts.ppd_page_size_letter),
            ("PageSize", "Legal", &texts.ppd_page_size_legal),
            ("PageRegion", "A4", &texts.ppd_page_size_a4),
            ("PageRegion", "Letter", &texts.ppd_page_size_letter),
            ("PageRegion", "Legal", &texts.ppd_page_size_legal),
            ("Duplex", "None", &texts.ppd_duplex_none),
            ("Duplex", "DuplexNoTumble", &texts.ppd_duplex_long_edge),
            ("Duplex", "DuplexTumble", &texts.ppd_duplex_short_edge),
        ] {
            if !text.is_empty() {
                translation_lines.push_str(&format!(
                    "*{lang}.{opt} {value}/{text}: \"\"\n",
                    lang = ppd_lang,
                    opt = opt,
                    value = value,
                    text = text
                ));
            }
        }
        translation_lines.push('\n');
    }

    let cups_languages = cups_langs.join(" ");

    format!(
        r#"*PPD-Adobe: "4.3"
*% DupriMan 手动双面虚拟打印机（由 dupriman install 生成，请勿手改）
*FormatVersion: "4.3"
*FileVersion: "1.0"
*LanguageVersion: English
*LanguageEncoding: ISOLatin1
*PCFileName: "DUPRIMAN.PPD"
*Manufacturer: "DupriMan"
*Product: "(DupriMan Manual Duplex)"
*ModelName: "{name}"
*ShortNickName: "{name}"
*NickName: "{info}"
*PSVersion: "(3010.000) 0"
*LanguageLevel: "3"
*ColorDevice: False
*DefaultColorSpace: Gray
*FileSystem: False
*Throughput: "1"
*LandscapeOrientation: Plus90
*DriverType Filter: ""
*cupsVersion: 2.2
*cupsModelNumber: 0
*cupsManualCopies: True
*cupsLanguages: "{cups_languages}"
*cupsFilter: "application/vnd.cups-pdf 0 -"
*cupsFilter2: "application/vnd.cups-pdf application/pdf 0 -"
*APPrinterIconPath: "/usr/share/cups/icons/printer.png"

*OpenUI *PageSize/Page Size: PickOne
*OrderDependency: 10 AnySetup *PageSize
*DefaultPageSize: A4
*PageSize A4/A4: "<</PageSize[595 842]/ImagingBBox null>>setpagedevice"
*PageSize Letter/Letter: "<</PageSize[612 792]/ImagingBBox null>>setpagedevice"
*PageSize Legal/Legal: "<</PageSize[612 1008]/ImagingBBox null>>setpagedevice"
*CloseUI: *PageSize

*OpenUI *PageRegion/Page Size: PickOne
*OrderDependency: 10 AnySetup *PageRegion
*DefaultPageRegion: A4
*PageRegion A4/A4: "<</PageSize[595 842]/ImagingBBox null>>setpagedevice"
*PageRegion Letter/Letter: "<</PageSize[612 792]/ImagingBBox null>>setpagedevice"
*PageRegion Legal/Legal: "<</PageSize[612 1008]/ImagingBBox null>>setpagedevice"
*CloseUI: *PageRegion

*DefaultImageableArea: A4
*ImageableArea A4/A4: "18 12 577 830"
*ImageableArea Letter/Letter: "18 12 594 780"
*ImageableArea Legal/Legal: "18 12 594 996"
*DefaultPaperDimension: A4
*PaperDimension A4/A4: "595 842"
*PaperDimension Letter/Letter: "612 792"
*PaperDimension Legal/Legal: "612 1008"

*OpenUI *Duplex/Duplex: PickOne
*OrderDependency: 20 AnySetup *Duplex
*DefaultDuplex: DuplexNoTumble
*Duplex None/Off (single-sided): ""
*Duplex DuplexNoTumble/On (long-edge): ""
*Duplex DuplexTumble/On (short-edge): ""
*CloseUI: *Duplex

*% ===== 翻译由 i18n 系统生成 =====
{translations}
*DefaultFont: Courier
*Font AvantGarde-Book: Standard "(001.006S)" Standard ROM
*Font Courier: Standard "(002.004S)" Standard ROM
*Font Helvetica: Standard "(001.006S)" Standard ROM
*Font Times-Roman: Standard "(002.006S)" Standard ROM
"#,
        name = name,
        info = info,
        cups_languages = cups_languages,
        translations = translation_lines,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppd_contains_key_entries() {
        let ppd = generate(&Config::default());
        assert!(ppd.contains("*cupsFilter: \"application/vnd.cups-pdf 0 -\""));
        assert!(ppd.contains("*cupsManualCopies: True"));
        assert!(ppd.contains("*Duplex DuplexNoTumble/On (long-edge)"));
        assert!(ppd.contains("*Duplex DuplexTumble/On (short-edge)"));
        // 简体中文翻译来自 i18n
        assert!(ppd.contains("*zh_CN.Translation Duplex/双面打印"));
        assert!(ppd.contains("*zh_CN.Duplex None/单面"));
        assert!(ppd.contains("*zh_CN.Duplex DuplexNoTumble/长边翻转"));
        assert!(ppd.contains("*zh_CN.Duplex DuplexTumble/短边翻转"));
        // 繁体中文翻译
        assert!(ppd.contains("*zh_TW.Translation Duplex/雙面列印"));
        assert!(ppd.contains("*zh_TW.Duplex DuplexNoTumble/長邊翻轉"));
        // cupsLanguages 包含所有语言（顺序不定）
        let cups_lang_line = ppd.lines()
            .find(|l| l.starts_with("*cupsLanguages:"))
            .unwrap();
        assert!(cups_lang_line.contains("en"), "missing en in: {cups_lang_line}");
        assert!(cups_lang_line.contains("zh_CN"), "missing zh_CN in: {cups_lang_line}");
        assert!(cups_lang_line.contains("zh_TW"), "missing zh_TW in: {cups_lang_line}");
    }
}
