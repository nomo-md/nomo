"""把 results/bench/*.jsonl 汇总成 Markdown 表格。用法：python3 -I scripts/summarize_bench.py results/bench"""

import json
import sys
from pathlib import Path


def load(path: Path) -> dict:
    data = {"phases": {}}
    for line in path.read_text(encoding="utf-8").splitlines():
        event = json.loads(line)
        if event["event"] == "phase":
            data["phases"][event["name"]] = event
        else:
            data[event["event"]] = event
    return data


def main() -> None:
    root = Path(sys.argv[1])
    print("| 样本 | 断行 | 行数 | 首帧(进程起) | 跳转首帧最大 | 匀速滚动 元素 p50/p95/max | 匀速滚动 帧间隔 p95/max | 掉帧 | 甩动 布局 p95/max | 输入 元素 p50/max | 末尾可达 | 撤销还原 | KP 单段最大 |")
    print("|---|---|---:|---:|---:|---|---|---:|---|---|---|---|---:|")
    for path in sorted(root.glob("*.jsonl")):
        data = load(path)
        phases = data["phases"]
        if "jump" not in phases:
            print(f"| {path.stem} | 未完成 |")
            continue
        name, mode = path.stem.rsplit("_", 1)
        scroll, fling, typing, undo, jump = (phases.get(key, {}) for key in ("scroll_40px", "fling_4000px", "type_mid", "undo", "jump"))
        fmt = lambda phase, key, *fields: "/".join(f"{phase[key][field]:.1f}" for field in fields) if phase else "—"
        print(
            f"| {name} | {mode} | {data['open']['lines']} | {data['first_frame']['since_process_start_ms']:.0f} ms"
            f" | {jump['element_ms']['max']:.1f} | {fmt(scroll, 'element_ms', 'p50', 'p95', 'max')}"
            f" | {fmt(scroll, 'interval_ms', 'p95', 'max')} | {scroll.get('janky_frames', '—')}/{scroll.get('frames', '—')}"
            f" | {fmt(fling, 'layout_ms', 'p95', 'max')} | {fmt(typing, 'element_ms', 'p50', 'max')}"
            f" | {jump.get('end_reached')} | {undo.get('end_reached')} | {(undo or jump)['kp']['max_ms']:.1f} |"
        )


if __name__ == "__main__":
    main()
