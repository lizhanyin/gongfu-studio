#!/usr/bin/env bash
# Gongfu Studio 清理 + 构建 + 启动（Git Bash / Linux 风格 shell）
# 用法: ./run.sh [--full]   --full 会先执行 cargo clean 全量重编
set -e
cd "$(dirname "$0")"

echo "[1/4] 清理: 结束残留进程..."
# Git Bash 下 taskkill 参数需要双斜杠，避免被当作路径转换
taskkill //IM Gongfu-studio.exe //F >/dev/null 2>&1 || true

echo "[2/4] 清理: 删除旧产物..."
rm -f target/release/Gongfu-studio.exe
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
cargo build --release

echo "[4/4] 启动 Gongfu Studio..."
./target/release/Gongfu-studio.exe &
disown 2>/dev/null || true
echo "完成。"
