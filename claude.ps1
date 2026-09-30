# ===== 基础连接 =====
$env:ANTHROPIC_BASE_URL = "http://100.121.131.101:8765"
$env:ANTHROPIC_AUTH_TOKEN = "workbuddy"
$env:ANTHROPIC_MODEL = "kimi-k2.7"

# ===== 上下文窗口（K2.7 Code 实际为 256K）=====
$env:CLAUDE_CODE_MAX_CONTEXT_TOKENS = "256000"
$env:CLAUDE_CODE_AUTO_COMPACT_WINDOW = "256000"

# ===== 自动压缩触发阈值 =====
$env:CLAUDE_AUTOCOMPACT_PCT_OVERRIDE = "70"

# ===== 单次最大输出 =====
$env:CLAUDE_CODE_MAX_OUTPUT_TOKENS = "32768"

# ===== 关闭非必要流量 =====
$env:CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = "1"

$env:CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING = "1"
$env:CLAUDE_CODE_EFFORT_LEVEL = "max"

claude