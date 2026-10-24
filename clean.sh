#!/usr/bin/env bash
# Gongfu Studio 清理全部构建产物（Git Bash / Linux 风格 shell）
# 用法: ./clean.sh [--deps]   --deps 同时删除 node_modules
set -e
cd "$(dirname "$0")"

# 真实产物名（Cargo 对包名 Gongfu-Studio 的规范化输出，注意小写 s）
EXE_NAME="Gongfu-Studio.exe"

app_running() {
    powershell -NoProfile -Command \
        "if (Get-Process -Name 'Gongfu-Studio' -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }" \
        >/dev/null 2>&1
}

echo "[1/3] 清理: 结束残留进程..."
# 运行中的 exe 锁着 target/ 下的文件，不先结束 cargo clean 会报难懂的错
if app_running; then
    powershell -NoProfile -Command \
        "Get-Process -Name 'Gongfu-Studio' -ErrorAction SilentlyContinue | Stop-Process -Force" \
        >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5; do
        app_running || break
        sleep 1
    done
    if app_running; then
        echo "错误: 无法结束 ${EXE_NAME}（可能被占用），请手动关闭后重试。" >&2
        exit 1
    fi
fi

echo "[2/3] 清理: 删除构建产物..."
cargo clean
rm -rf gui/target gui/ui/dist
# wasm-pack 产物未入库，npm run build 的 wasm 脚本会重新生成
rm -rf gui/ui/src/wasm

if [ "$1" = "--deps" ]; then
    echo "[2/3] --deps: 删除 node_modules..."
    rm -rf gui/ui/node_modules cdp/node_modules
fi

echo "[3/3] 完成。下次 run.bat / run.sh 会全量重建。"
