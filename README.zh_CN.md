# DupriMan — 手动双面打印助手

[English](README.md) | **简体中文** | [繁體中文](README.zh_TW.md)

为**单面打印机**提供近乎自动的双面打印体验：
在任意应用的打印对话框中，选择虚拟打印机 **DupriMan-Duplex**，并选择
**关闭 / 长边翻转 / 短边翻转** 双面模式。它将：

1. 打印**所有正面**（奇数页，全部份数）；
2. 弹出 GTK 对话框，提示重新放入纸张；
3. 点击**确定**后打印**所有背面**（偶数页）。

## 架构

```mermaid
flowchart LR
    A[应用程序<br/>打印到 DupriMan-Duplex] -->|PDF + 双面选项| B[CUPS 队列<br/>dupriman://]
    B --> C[dupriman 后端<br/>CUPS 后端进程]
    C -->|qpdf 拆分| D[正面 PDF<br/>奇数页]
    C -->|qpdf 拆分 + 倒序/旋转| E[背面 PDF<br/>偶数页]
    D --> F[真实打印机队列]
    C -->|写入请求到 /run/dupriman| G[dupriman prompt<br/>GTK 弹窗守护进程<br/>systemd --user]
    G -->|用户点击确定| C
    E --> F
```

- **后端**（`dupriman backend`，由 cupsd 调用）：解析双面选项，用 qpdf 拆分奇偶页，提交到真实打印机队列，并在继续之前**等待物理打印完成**。
- **弹窗守护进程**（`dupriman prompt`，运行在用户图形会话中）：CUPS 后端以 `cups` 用户身份运行，无法显示对话框，因此通过 `/run/dupriman` 中的请求/响应文件进行通信。守护进程显示 GTK4 对话框。取消或超时会安全中止背面打印任务。
- **多份拷贝**：PPD 声明了 `cupsManualCopies`，因此份数会透传到真实队列——先打印全部正面（×N），弹窗一次，再打印全部背面（×N）。

## 依赖

运行时：`cups`、`qpdf`、`poppler`（pdfinfo）、`gtk4`（≥ 4.10）、`systemd`
构建：`rust` / `cargo`、`pkg-config`

Arch Linux：

```bash
sudo pacman -S --needed cups qpdf poppler gtk4 rust
```

## 安装

### 从源码安装

```bash
cargo build --release
sudo ./target/release/dupriman install
```

`install` 按 FHS 规范放置文件并激活：

| 路径 | 说明 |
|---|---|
| `/usr/bin/dupriman` | 主程序 |
| `/usr/lib/cups/backend/dupriman` | CUPS 后端（符号链接） |
| `/usr/share/cups/model/dupriman.ppd` | 虚拟打印机 PPD |
| `/etc/dupriman/config.toml` | 系统级配置（已存在时不覆盖） |
| `/usr/lib/systemd/user/dupriman-prompt.service` | 弹窗守护进程（全局启用） |
| `/usr/lib/tmpfiles.d/dupriman.conf` | 状态/暂存目录 |

然后启动弹窗服务（或重新登录）：

```bash
systemctl --user start dupriman-prompt
dupriman test   # 环境自检
```

## 配置

系统级：`/etc/dupriman/config.toml`；用户级覆盖：`~/.config/dupriman/config.toml`
（只需写入要覆盖的字段）。完整注释示例：`dupriman sample-config`。

关键选项（默认值适用于大多数出纸面朝下的激光打印机）：

```toml
[printer]
# 特殊值 "save-pdf"：不打印，子作业保存到 spool_dir/saved/（测试模式，默认）
# 真机打印时改为实际队列名，如 "HP_LaserJet_1020"（lpstat -v 查看）
real = "save-pdf"

[duplex]
even_order = "reverse"           # 背面页序：reverse（倒序）| normal（正序）

[duplex.long_edge]
even_rotate = 180                # 长边翻转：背面旋转角度

[duplex.short_edge]
even_rotate = 0                  # 短边翻转：背面旋转角度

[prompt]
language = "auto"                # 对话框语言：auto | en | zh_CN | zh_TW
timeout_secs = 600               # 0 = 永久等待；超时后中止
```

### 国际化（i18n）

对话框文本由 `prompt.language` 选择：

| 值 | 语言 |
|---|---|
| `auto` | 从 `LANG` / `LC_ALL` 自动检测（默认） |
| `en` / `en_US` | 英语 |
| `zh_CN` | 简体中文 |
| `zh_TW` | 繁体中文 |

也可以直接覆盖任意文本：

```toml
[prompt]
title = "自定义标题"
message = "请重新放入纸张"
ok_label = "继续"
cancel_label = "停止"
```

自定义文本始终优先于语言默认值。

### 方向或页序不对？

默认值假设**出纸面朝下的输出托盘**，用户操作是
**从输出托盘取出整叠纸，原样放回进纸槽**：

| 症状 | 修复方法 |
|---|---|
| 背面页序反了（最后一页在最上面） | `duplex.even_order = "normal"` |
| 背面颠倒（长边翻转） | `duplex.long_edge.even_rotate = 0` |
| 背面颠倒（短边翻转） | `duplex.short_edge.even_rotate = 180` |
| 出纸面朝上的打印机（如部分喷墨） | `duplex.even_order = "normal"` + 对调两个 `even_rotate` 值 |

建议先用 3–4 页的文档测试。

## 使用方法

在打印对话框中选择 **DupriMan-Duplex**，然后选择双面模式：

- **关闭**：直接转发到真实打印机；
- **长边翻转**（默认）：适用于纵向文档，沿长边翻转（背面旋转 180°）；
- **短边翻转**：适用于横向文档，沿短边翻转（背面不旋转）。

正面打印完成后，**不要翻转或重新排序**——从输出托盘取出整叠纸，原样放入进纸槽，然后点击确定。

## 日志与故障排除

- 后端日志：`journalctl -u cups` 或 `/var/log/cups/error_log`（前缀 `[dupriman]`）
- 弹窗守护进程：`journalctl --user -u dupriman-prompt`
- 自检：`dupriman test`
- 如果请求双面打印时弹窗守护进程未运行：正面会正常打印，之后后端等待并在 `prompt.timeout_secs` 后安全中止。
- 调试拆分结果：设置 `print.keep_temp = true`；临时 PDF 将保留在 `paths.spool_dir` 中。

## 卸载

```bash
sudo dupriman uninstall            # 移除打印机/服务/系统文件，保留配置
sudo dupriman uninstall --purge    # 同时移除配置和二进制文件
```

## 开发

```bash
cargo test            # 单元测试 + 真实 qpdf 拆分测试
tests/e2e.sh          # 端到端模拟（mock lp/lpstat，不浪费纸张）
cupstestppd <(./target/debug/dupriman ppd)   # 验证 PPD
```

### 添加新语言

1. 从 `i18n/en_US.toml` 复制创建 `i18n/<locale>.toml`，翻译所有键。
2. 在 `src/i18n.rs` 中注册（全部集中在一处）：
   - `Lang` 枚举：添加变体，带上 serde rename 和 alias
   - `Lang::ALL`：加入新变体
   - `Lang::from_env()`：添加从 `LANG`/`LC_ALL` 检测语言的逻辑
   - `Lang::file_stem()`：映射到语言文件名
   - `Lang::raw_toml()`：添加 `include_str!("../i18n/<stem>.toml")` 分支
3. 更新[国际化](#国际化i18n)中的语言表（可选）。

缺失的键会自动回退到 `en_US`。`install` 通过 `Lang::ALL` 自动发现所有语言，无需修改安装脚本。

### 已测试硬件

本软件仅在 **Arch Linux** 上的 **HP LaserJet 1020** + **foo2zjs** 驱动上测试过。
如果你在其他打印机型号或驱动上测试，欢迎反馈结果或提交 PR，
尤其是适合你设备的 `even_order` 和 `even_rotate` 配置值。
