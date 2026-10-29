"""生成固定内容的百万字级混合 Markdown，用于帧耗时与按键上屏的同场景前后对照。

内容只由种子决定：同一 --chars 与 --seed 每次生成逐字节相同的文件。
包含标题、段落、行内格式、列表、待办、引用、提示块、代码块和表格，不含图片与外链。
"""
from pathlib import Path
import argparse
import random

WORDS = "同步 文档 编辑 滚动 目录 表格 段落 引用 标题 列表 性能 渲染 输入 光标 选区 保存 本地 结构 索引 片段".split()
CALLOUTS = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]


def sentence(rng: random.Random) -> str:
    words = [rng.choice(WORDS) for _ in range(rng.randint(8, 18))]
    if rng.random() < 0.3:
        index = rng.randrange(len(words))
        words[index] = rng.choice(["**{}**", "*{}*", "`{}`", "~~{}~~"]).format(words[index])
    return "".join(words) + "。"


def section(rng: random.Random, number: int) -> str:
    parts = [f"## 第 {number} 节 {rng.choice(WORDS)}与{rng.choice(WORDS)}\n"]
    for _ in range(rng.randint(2, 4)):
        parts.append("".join(sentence(rng) for _ in range(rng.randint(3, 6))) + "\n")
    kind = number % 6
    if kind == 0:
        parts.append("\n".join(f"- {sentence(rng)}" for _ in range(5)) + "\n")
    elif kind == 1:
        parts.append("\n".join(f"- [{'x' if rng.random() < 0.5 else ' '}] {sentence(rng)}" for _ in range(4)) + "\n")
    elif kind == 2:
        parts.append(f"> [!{CALLOUTS[number % len(CALLOUTS)]}]\n> {sentence(rng)}\n")
    elif kind == 3:
        rows = "\n".join(f"| {rng.choice(WORDS)} | {rng.randint(1, 999)} | {rng.choice(WORDS)} |" for _ in range(6))
        parts.append(f"| 项目 | 数值 | 备注 |\n| --- | --- | --- |\n{rows}\n")
    elif kind == 4:
        body = "\n".join(f"let value_{line} = {rng.randint(0, 9999)}; // {rng.choice(WORDS)}" for line in range(8))
        parts.append(f"```rust\n{body}\n```\n")
    else:
        parts.append(f"> {sentence(rng)}\n")
    return "\n".join(parts) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--chars", type=int, default=1_000_000, help="目标字符数（按 Unicode 字符计，生成结果略大于该值）")
    parser.add_argument("--seed", type=int, default=20261028)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    chunks, total, number = ["# 百万字性能基准文档\n\n"], 0, 1
    while total < args.chars:
        chunk = section(rng, number)
        chunks.append(chunk)
        total += len(chunk)
        number += 1
    args.output.write_text("".join(chunks), encoding="utf-8")
    print(f"{args.output}: {total} 字符, {number - 1} 节")


if __name__ == "__main__":
    main()
