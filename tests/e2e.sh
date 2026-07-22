#!/usr/bin/env bash
# DupriMan 后端端到端模拟测试：
# 用 mock 的 lp/lpstat/cancel 替代真实打印机，验证双面流程：
#   正面(全部份数) -> 弹窗请求 -> 模拟用户确认 -> 背面(全部份数)
# 用法: tests/e2e.sh [dupriman 二进制路径]
set -u

BIN="${1:-./target/debug/dupriman}"
WORK="$(mktemp -d /tmp/dupriman-e2e.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/mockbin" "$WORK/state" "$WORK/spool"

# ---- mock lp：记录调用，返回假作业号 ----
cat > "$WORK/mockbin/lp" <<'EOF'
#!/usr/bin/env bash
echo "LP $@" >> "$MOCK_LOG"
echo "request id is MOCKPRN-$RANDOM (1 file(s))"
EOF

# ---- mock lpstat：队列恒为空（作业立即完成）----
cat > "$WORK/mockbin/lpstat" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

# ---- mock cancel：记录 ----
cat > "$WORK/mockbin/cancel" <<'EOF'
#!/usr/bin/env bash
echo "CANCEL $@" >> "$MOCK_LOG"
EOF
chmod +x "$WORK/mockbin/"{lp,lpstat,cancel}

# ---- 生成 7 页测试 PDF（用 dupriman 测试夹具同款结构的简化版，这里直接用 python）----
python3 - "$WORK/doc.pdf" <<'EOF'
import sys
pages, out = 7, sys.argv[1]
buf = bytearray(b"%PDF-1.4\n")
offsets = [0]
def obj(body):
    offsets.append(len(buf))
    buf.extend(f"{len(offsets)-1} 0 obj\n{body}\nendobj\n".encode())
obj("<< /Type /Catalog /Pages 2 0 R >>")
kids = " ".join(f"{3+i} 0 R" for i in range(pages))
obj(f"<< /Type /Pages /Kids [{kids}] /Count {pages} /MediaBox [0 0 595 842] >>")
for _ in range(pages):
    obj("<< /Type /Page /Parent 2 0 R /Resources << >> >>")
xref = len(buf)
buf.extend(f"xref\n0 {len(offsets)}\n".encode())
buf.extend(b"0000000000 65535 f \n")
for o in offsets[1:]:
    buf.extend(f"{o:010} 00000 n \n".encode())
buf.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
open(out, "wb").write(bytes(buf))
EOF

# ---- 覆盖配置：save-pdf 测试模式 + 中文界面 ----
cat > "$WORK/config.toml" <<EOF
[printer]
real = "save-pdf"
[paths]
state_dir = "$WORK/state"
spool_dir = "$WORK/spool"
[prompt]
enabled = true
timeout_secs = 30
language = "zh_CN"
[print]
poll_interval_ms = 50
EOF

# ---- 模拟弹窗守护：看到请求立即"点击确定" ----
(
  for _ in $(seq 1 100); do
    for f in "$WORK/state"/req-*.json; do
      [ -e "$f" ] || continue
      id=$(basename "$f" .json); id=${id#req-}
      printf '{"id":"%s","confirmed":true,"reason":""}' "$id" > "$WORK/state/resp-$id.json"
      rm -f "$f"
    done
    sleep 0.1
  done
) &
WATCHER=$!

export MOCK_LOG="$WORK/calls.log"
export PATH="$WORK/mockbin:$PATH"

echo "=== 场景 1：长边翻转 2 份拷贝 ==="
"$BIN" --config "$WORK/config.toml" backend 100 alice 报告 2 "Duplex=DuplexNoTumble" "$WORK/doc.pdf"
rc=$?
kill $WATCHER 2>/dev/null; wait $WATCHER 2>/dev/null

echo "退出码: $rc"
echo "--- lp 调用记录 ---"
cat "$MOCK_LOG"

pass=1
[ $rc -eq 0 ] || { echo "FAIL: 退出码非 0"; pass=0; }
grep -q '（正面）' "$MOCK_LOG" || { echo "FAIL: 缺少正面打印"; pass=0; }
grep -q '（背面）' "$MOCK_LOG" || { echo "FAIL: 缺少背面打印"; pass=0; }
[ "$(grep -c 'LP ' "$MOCK_LOG")" -eq 2 ] || { echo "FAIL: lp 调用次数应为 2"; pass=0; }
grep -q 'LP -d MOCKPRN.*-n 2' "$MOCK_LOG" || { echo "FAIL: 份数未传递"; pass=0; }
# 正面在前、背面在后
front_line=$(grep -n '正面' "$MOCK_LOG" | head -1 | cut -d: -f1)
back_line=$(grep -n '背面' "$MOCK_LOG" | head -1 | cut -d: -f1)
[ "$front_line" -lt "$back_line" ] || { echo "FAIL: 打印顺序错误"; pass=0; }
# 确认拆出的 PDF 已被清理（keep_temp=false）
[ -z "$(ls -A "$WORK/spool" 2>/dev/null)" ] || { echo "FAIL: 临时文件未清理"; pass=0; }

echo "=== 场景 2：单面直通 ==="
: > "$MOCK_LOG"
"$BIN" --config "$WORK/config.toml" backend 101 alice 单页文档 1 "Duplex=None" "$WORK/doc.pdf"
rc=$?
[ $rc -eq 0 ] && [ "$(grep -c 'LP ' "$MOCK_LOG")" -eq 1 ] || { echo "FAIL: 单面应只调用一次 lp"; pass=0; }
grep -q '（正面）\|（背面）' "$MOCK_LOG" && { echo "FAIL: 单面不应拆分"; pass=0; }
cat "$MOCK_LOG"

echo "=== 场景 3：短边翻转（背面需旋转 180）==="
: > "$MOCK_LOG"
(
  for _ in $(seq 1 100); do
    for f in "$WORK/state"/req-*.json; do
      [ -e "$f" ] || continue
      id=$(basename "$f" .json); id=${id#req-}
      printf '{"id":"%s","confirmed":true,"reason":""}' "$id" > "$WORK/state/resp-$id.json"
      rm -f "$f"
    done
    sleep 0.1
  done
) &
WATCHER=$!
"$BIN" --config "$WORK/config.toml" backend 102 alice 竖版 1 "sides=two-sided-short-edge" "$WORK/doc.pdf"
rc=$?
kill $WATCHER 2>/dev/null; wait $WATCHER 2>/dev/null
[ $rc -eq 0 ] || { echo "FAIL: 短边翻转退出码非 0"; pass=0; }
cat "$MOCK_LOG"

echo "=== 场景 4：用户取消（无守护响应，缩短超时）==="
cat > "$WORK/config2.toml" <<EOF
[printer]
real = "MOCKPRN"
[paths]
state_dir = "$WORK/state"
spool_dir = "$WORK/spool"
[prompt]
enabled = true
timeout_secs = 1
[print]
poll_interval_ms = 50
EOF
: > "$MOCK_LOG"
"$BIN" --config "$WORK/config2.toml" backend 103 alice 取消测试 1 "Duplex=DuplexNoTumble" "$WORK/doc.pdf" 2>"$WORK/stderr.log"
rc=$?
[ $rc -ne 0 ] || { echo "FAIL: 超时应非 0 退出"; pass=0; }
grep -q '超时' "$WORK/stderr.log" || { echo "FAIL: 应有超时提示"; pass=0; }
# 取消后不应有背面打印
grep -q '背面' "$MOCK_LOG" && { echo "FAIL: 取消后不应打印背面"; pass=0; }
echo "（预期失败，退出码 $rc）"
grep 'ERROR' "$WORK/stderr.log" | head -2

if [ $pass -eq 1 ]; then
  echo; echo "E2E 全部通过 ✔"
else
  echo; echo "E2E 存在失败项 ✘"
  exit 1
fi
