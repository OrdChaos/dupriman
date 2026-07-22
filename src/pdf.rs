//! PDF 处理：页数统计、奇偶页拆分、旋转。调用系统工具 pdfinfo / qpdf。

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::config::EvenOrder;

/// 用 pdfinfo 获取页数
pub fn page_count(path: &Path) -> Result<u32> {
    let out = Command::new("pdfinfo")
        .arg(path)
        .output()
        .context("无法执行 pdfinfo（需要安装 poppler）")?;
    if !out.status.success() {
        bail!(
            "pdfinfo 解析失败 {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            return rest
                .trim()
                .parse::<u32>()
                .context("pdfinfo 页数格式异常");
        }
    }
    bail!("pdfinfo 输出中未找到 Pages 字段: {}", path.display());
}

/// 用 qpdf 按页码列表抽取子文档
pub fn extract(input: &Path, pages: &[u32], output: &Path) -> Result<()> {
    let list = pages
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let input_s = input
        .to_str()
        .with_context(|| format!("输入路径含非 UTF-8 字符: {}", input.display()))?;
    let output_s = output
        .to_str()
        .with_context(|| format!("输出路径含非 UTF-8 字符: {}", output.display()))?;
    let out = Command::new("qpdf")
        .args(["--empty", "--pages", input_s, &list, "--", output_s])
        .output()
        .context("无法执行 qpdf")?;
    check_qpdf_status(&out, &format!("拆分页码 {list}"))
}

/// qpdf 退出码：0=成功 2=错误 3=成功但有警告。警告不视为失败。
fn check_qpdf_status(out: &std::process::Output, what: &str) -> Result<()> {
    match out.status.code() {
        Some(0) => Ok(()),
        Some(3) => {
            eprintln!(
                "INFO: [dupriman] qpdf {what} 完成但有警告: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            Ok(())
        }
        _ => bail!(
            "qpdf {what} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// 用 qpdf 把 b 拼接到 a 之后（a 的页在前）
pub fn merge(a: &Path, b: &Path, output: &Path) -> Result<()> {
    let s = |p: &Path| -> Result<String> {
        p.to_str()
            .map(str::to_string)
            .with_context(|| format!("路径含非 UTF-8 字符: {}", p.display()))
    };
    let out = Command::new("qpdf")
        .args(["--empty", "--pages", &s(a)?, &s(b)?, "--", &s(output)?])
        .output()
        .context("无法执行 qpdf")?;
    check_qpdf_status(&out, "合并 PDF")
}

/// 用 qpdf 将文档全部页面原地旋转指定角度（0/90/180/270）
pub fn rotate_all(path: &Path, angle: i32) -> Result<()> {
    let angle = angle.rem_euclid(360);
    if angle == 0 {
        return Ok(());
    }
    let spec = format!("--rotate=+{angle}:1-z");
    let out = Command::new("qpdf")
        .arg(spec)
        .arg(path)
        .arg("--replace-input")
        .output()
        .context("无法执行 qpdf")?;
    check_qpdf_status(&out, &format!("旋转 {}", path.display()))
}

/// 奇数页页码列表（正面）：1, 3, 5, ...
pub fn odd_pages(total: u32) -> Vec<u32> {
    (1..=total).step_by(2).collect()
}

/// 生成一个单页空白 PDF（A4），用于奇数页文档背面补齐。
/// 页面完全空白（空内容流），打印时不会留下任何痕迹，
/// 让最后一张纸参与背面走纸，从而能一次性从出纸口取走全部纸张。
pub fn blank_page(path: &Path) -> Result<()> {
    use std::io::Write;
    let mut buf: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets: Vec<usize> = vec![0];
    let mut push = |body: &str| {
        offsets.push(buf.len());
        let num = offsets.len() - 1;
        writeln!(buf, "{num} 0 obj\n{body}\nendobj").unwrap();
    };
    push("<< /Type /Catalog /Pages 2 0 R >>");
    push("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>");
    push("<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 4 0 R >>");
    push("<< /Length 0 >>\nstream\n\nendstream");
    let xref = buf.len();
    let count = offsets.len();
    writeln!(buf, "xref\n0 {count}").unwrap();
    writeln!(buf, "0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(buf, "{off:010} 00000 n ").unwrap();
    }
    writeln!(
        buf,
        "trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    )
    .unwrap();
    std::fs::write(path, &buf)?;
    Ok(())
}

/// 偶数页页码列表（背面），可按配置倒序（出纸面朝下的激光机通常需要）
pub fn even_pages(total: u32, order: EvenOrder) -> Vec<u32> {
    let mut pages: Vec<u32> = (2..=total).step_by(2).collect();
    if order == EvenOrder::Reverse {
        pages.reverse();
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn odd_even_lists() {
        assert_eq!(odd_pages(7), vec![1, 3, 5, 7]);
        assert_eq!(even_pages(7, EvenOrder::Normal), vec![2, 4, 6]);
        assert_eq!(even_pages(7, EvenOrder::Reverse), vec![6, 4, 2]);
        assert_eq!(odd_pages(1), vec![1]);
        assert!(even_pages(1, EvenOrder::Normal).is_empty());
        assert_eq!(even_pages(8, EvenOrder::Reverse), vec![8, 6, 4, 2]);
    }

    /// 生成一个最小可解析的 n 页空白 PDF（供集成测试）
    pub fn make_test_pdf(dir: &Path, name: &str, pages: u32) -> PathBuf {
        let path = dir.join(name);
        let mut buf: Vec<u8> = b"%PDF-1.4\n".to_vec();
        let mut offsets: Vec<usize> = vec![0]; // 对象 0 不使用

        let push_obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: String| {
            offsets.push(buf.len());
            let num = offsets.len() - 1;
            write!(buf, "{num} 0 obj\n{body}\nendobj\n").unwrap();
        };

        push_obj(&mut buf, &mut offsets, "<< /Type /Catalog /Pages 2 0 R >>".into());
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
        push_obj(
            &mut buf,
            &mut offsets,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 595 842] >>",
                kids.join(" ")
            ),
        );
        for _ in 0..pages {
            push_obj(
                &mut buf,
                &mut offsets,
                "<< /Type /Page /Parent 2 0 R /Resources << >> >>".into(),
            );
        }

        let xref_pos = buf.len();
        let count = offsets.len();
        writeln!(buf, "xref\n0 {count}").unwrap();
        writeln!(buf, "0000000000 65535 f ").unwrap();
        for off in offsets.iter().skip(1) {
            writeln!(buf, "{off:010} 00000 n ").unwrap();
        }
        writeln!(
            buf,
            "trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF"
        )
        .unwrap();

        std::fs::write(&path, buf).unwrap();
        path
    }

    fn tools_available() -> bool {
        Command::new("pdfinfo").arg("-v").output().is_ok()
            && Command::new("qpdf").arg("--version").output().is_ok()
    }

    #[test]
    fn split_and_rotate_real_pdf() {
        if !tools_available() {
            eprintln!("跳过：pdfinfo/qpdf 不可用");
            return;
        }
        let dir = std::env::temp_dir().join(format!("dupriman-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = make_test_pdf(&dir, "src.pdf", 7);

        assert_eq!(page_count(&src).unwrap(), 7);

        let odd = dir.join("odd.pdf");
        extract(&src, &odd_pages(7), &odd).unwrap();
        assert_eq!(page_count(&odd).unwrap(), 4);

        let even = dir.join("even.pdf");
        extract(&src, &even_pages(7, EvenOrder::Normal), &even).unwrap();
        assert_eq!(page_count(&even).unwrap(), 3);

        rotate_all(&even, 180).unwrap();
        assert_eq!(page_count(&even).unwrap(), 3); // 旋转不改变页数

        // 奇数页文档：补空白页后背面 = 偶数页数 + 1
        let blank = dir.join("blank.pdf");
        blank_page(&blank).unwrap();
        assert_eq!(page_count(&blank).unwrap(), 1);
        let full = dir.join("back-full.pdf");
        merge(&even, &blank, &full).unwrap();
        assert_eq!(page_count(&full).unwrap(), 4);

        std::fs::remove_dir_all(&dir).ok();
    }
}
