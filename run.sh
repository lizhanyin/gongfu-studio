#!/usr/bin/env bash
# Gongfu Studio 清理 + 构建 + 启动（Git Bash / Linux 风格 shell）
# 用法: ./run.sh [--full]   --full 会先执行 cargo clean 全量重编
set -e
cd "$(dirname "$0")"

# 真实产物名（Cargo 对包名 Gongfu-Studio 的规范化输出，注意小写 s）
EXE_NAME="Gongfu-Studio.exe"

app_running() {
    powershell -NoProfile -Command \
        "if (Get-Process -Name 'Gongfu-Studio' -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }" \
        >/dev/null 2>&1
}

echo "[1/4] 清理: 结束残留进程..."
# 用 PowerShell 而非 taskkill：taskkill 的 //IM 写法依赖 Git Bash 的 MSYS 路径
# 转换，换到 WSL 等环境会因参数原样透传而静默失效
if app_running; then
    powershell -NoProfile -Command \
        "Get-Process -Name 'Gongfu-Studio' -ErrorAction SilentlyContinue | Stop-Process -Force" \
        >/dev/null 2>&1 || true
    # 等待退出（最多 ~5s）。不校验的话，运行中的 exe 删不掉也链接不了，
    # rm -f 与 cargo 会静默/间接失败，报出难懂的链接错误
    for _ in 1 2 3 4 5; do
        app_running || break
        sleep 1
    done
    if app_running; then
        echo "错误: 无法结束 ${EXE_NAME}（可能被占用），请手动关闭后重试。" >&2
        exit 1
    fi
fi

echo "[2/4] 清理: 删除旧产物..."
rm -f "target/debug/$EXE_NAME"
rm -rf gui/ui/dist

if [ "$1" = "--full" ]; then
    echo "[2/4] --full: cargo clean 全量清理..."
    cargo clean
fi

echo "[3/4] 依赖检查..."
if [ ! -d gui/ui/node_modules ]; then
    npm --prefix gui/ui install
fi

echo "[3/4] 构建前端 (gui/ui)..."
npm --prefix gui/ui run build

echo "[3/4] 构建 Rust..."
cargo build

echo "[4/4] 启动 Gongfu Studio..."
./target/debug/"$EXE_NAME" &
disown 2>/dev/null || true
echo "完成。"
