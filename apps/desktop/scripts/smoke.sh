#!/usr/bin/env bash
# 冒烟：先构建 workspace release 二进制（T6 审查教训：fresh clone 上 sidecar 资源缺失会让 tauri build 失败）
# 再起 share → 校验核心端点 → 退出清理
set -euo pipefail
PORT="${1:-17952}"
cd "$(dirname "$0")/../../.."   # repo root

# bioinfo7 的系统 cargo (1.75) 不支持 resolver=3，优先用 rustup 的 cargo
CARGO=cargo
if [ -x "$HOME/.cargo/bin/cargo" ]; then
    CARGO="$HOME/.cargo/bin/cargo"
fi

# share 要求 --database 指向已存在的索引库，缺失时直接退出：
# 提示先跑一次 discover 生成索引
DB="$HOME/.fan-files/data/index.db"
if [ ! -f "$DB" ]; then
    echo "错误：未找到索引数据库 $DB" >&2
    echo "先跑一次 fan-files discover 生成索引" >&2
    exit 1
fi

PID=""
trap 'if [ -n "${PID:-}" ]; then kill "$PID" 2>/dev/null || true; fi' EXIT

echo "== build sidecars =="
"$CARGO" build --release -p fan-files -p fan-files-share

echo "== start share on $PORT =="
./target/release/fan-files-share --bind "127.0.0.1:$PORT" --database "$DB" &
PID=$!
sleep 2

curl -sf "http://127.0.0.1:$PORT/healthz" | grep -q '"ok"'
curl -sf "http://127.0.0.1:$PORT/api/v1/stats"
curl -sf "http://127.0.0.1:$PORT/api/v1/datasets?limit=5"
curl -sf "http://127.0.0.1:$PORT/api/v1/search?q=genome"
echo
echo "SMOKE OK"
