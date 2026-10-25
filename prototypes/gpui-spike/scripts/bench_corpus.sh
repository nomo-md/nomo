#!/usr/bin/env bash
# 用同一测试集逐个运行逐帧基准，每个样本一行 JSONL 输出到 results/bench/。
set -u
corpus="$1"
out="results/bench"
mkdir -p "$out"
bin="./target/release/nomo-gpui-spike"
samples=(
  "05_纯文本体量阶梯/10MiB_纯文本.md"
  "05_纯文本体量阶梯/50MiB_纯文本.md"
  "06_综合混合压力/10MiB_全节点混合.md"
  "06_综合混合压力/05MiB_全节点混合.md"
  "02_表格专项/04_极限长表格_10000行12列.md"
  "04_代码公式与图表/03_单个25000行代码块.md"
  "08_边界与兼容/01_单段落1MiB.md"
  "08_边界与兼容/02_单行64KiB不可断字符串.md"
  "01_基础与节点/03_2000标题与目录.md"
  "01_基础与节点/02_行内格式与多语言.md"
)
for sample in "${samples[@]}"; do
  for mode in kp greedy; do
    flag=""; [ "$mode" = greedy ] && flag="--greedy"
    name="$(basename "$sample" .md)_$mode"
    timeout 300 "$bin" "$corpus/$sample" --bench $flag 2>&1 | grep '"event"' > "$out/$name.jsonl"
    echo "$name exit=${PIPESTATUS[0]} lines=$(wc -l < "$out/$name.jsonl")"
  done
done
