"""按既有测试报告口径采样原型进程内存：冷启动打开单文件，每秒读取 phys_footprint，持续 25 秒。

phys_footprint 通过 libproc 的 proc_pid_rusage(RUSAGE_INFO_V2) 读取，与活动监视器“内存”列同源；
稳定值取最后 5 次采样均值，峰值取全程最大值。原型是单进程，没有 WebContent 子进程。

用法：python3 -I scripts/sample_memory.py <binary> <corpus_dir> <output.md>
"""

import ctypes
import subprocess
import sys
import time
from pathlib import Path

SAMPLES = [
    "01_基础与节点/00_真正空白.md",
    "01_基础与节点/10_短文编辑基线.md",
    "02_表格专项/04_极限长表格_10000行12列.md",
    "06_综合混合压力/01MiB_全节点混合.md",
    "06_综合混合压力/05MiB_全节点混合.md",
    "06_综合混合压力/10MiB_全节点混合.md",
    "05_纯文本体量阶梯/10MiB_纯文本.md",
    "05_纯文本体量阶梯/50MiB_纯文本.md",
]
DURATION_S = 25
# rusage_info_v2：16 字节 UUID 后依次为 8 个 uint64，phys_footprint 是第 8 个。
PHYS_FOOTPRINT_OFFSET = 16 + 7 * 8
RUSAGE_INFO_V2 = 2

libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
libproc.proc_pid_rusage.restype = ctypes.c_int


def phys_footprint(pid: int) -> int | None:
    buffer = ctypes.create_string_buffer(512)
    if libproc.proc_pid_rusage(pid, RUSAGE_INFO_V2, buffer) != 0:
        return None
    return int.from_bytes(buffer.raw[PHYS_FOOTPRINT_OFFSET : PHYS_FOOTPRINT_OFFSET + 8], "little")


def measure(binary: str, path: Path) -> tuple[float, float]:
    process = subprocess.Popen(
        [binary, str(path), "--exit-after", str(DURATION_S + 3)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    values = []
    try:
        for _ in range(DURATION_S):
            time.sleep(1)
            value = phys_footprint(process.pid)
            if value is not None:
                values.append(value / 1024 / 1024)
    finally:
        process.terminate()
        process.wait(timeout=10)
    if not values:
        return float("nan"), float("nan")
    tail = values[-5:]
    return max(values), sum(tail) / len(tail)


def main() -> None:
    binary, corpus, output = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    rows = ["| 样本 | 峰值 MiB | 稳定 MiB |", "|---|---:|---:|"]
    for sample in SAMPLES:
        peak, stable = measure(binary, corpus / sample)
        rows.append(f"| {Path(sample).name} | {peak:.0f} | {stable:.0f} |")
        print(rows[-1], flush=True)
    output.write_text("\n".join(rows) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
