# 首屏可见性能测试

README 展示的是 2026-11-07 最近两批 Windows 测试中的六类主要场景，均使用同一构建，每项运行三轮。指标为从已运行应用收到打开请求，到首屏文字出现的时间；应用启动不在计时范围内。

English: the README reports median time to first visible content for six scenarios, measured in three rounds on the same Windows build on November 7, 2026. Timing starts with an open request in an already running app and ends when the first screen's text appears; app startup is excluded.

## 测试环境

- 硬件：Intel Core i5-12400、Intel UHD Graphics 730，系统报告内存 16,908,595,200 字节。
- 系统与显示：Windows build 26200，1280×820 逻辑像素，100% 缩放，Light。
- 构建：Release，启用 `automation`，GPUI 0.3.10；构建标识 `native-release-3e2b199dc6ec`。
- 二进制 SHA-256：`3e2b199dc6ec5b3dbbc97b0be30cb19abcf7d2d14cd23bc52f0536edd040fd50`。
- 每轮独立进程与数据目录。单文档场景先显示轻量等待文档，再打开目标；多标签场景测已有三份文档时打开第四份。没有清空系统文件缓存。

## 首屏可见结果

单位为 ms。README 使用四舍五入后的三轮中位数。

| 测试场景 | 中位数 | 最小–最大 |
| --- | ---: | ---: |
| 5 MiB 纯文本 Markdown | **160** | 149–178 |
| 10 MiB 全节点混合 Markdown | **203** | 187–212 |
| 图片瀑布流 | **119** | 118–119 |
| 七类 Mermaid 图表 | **137** | 97–144 |
| 29 种代码语言 | **216** | 212–249 |
| 多标签：打开第 4 份文档 | **146** | 121–147 |

首屏文字由前台桌面像素匹配识别，计时包含文件 RPC、控制轮询和离散采样误差。该指标描述开始看到内容的速度，不表示整篇文档及全部图片、公式、图表已经渲染完成。

## 数据来源与复核

[精简原始数据](./performance/windows-opening-2026-11-07.json)保留这六个场景的逐轮首屏可见时间、汇总值、环境、构建与源码指纹、样本集及采样器指纹，并移除本机绝对路径和不相关诊断。

- `opening-2026-11-07T08-25-36-972Z`：纯文本、图片、Mermaid、代码、多标签场景。
- `opening-2026-11-07T08-43-09-992Z`：10 MiB 全节点混合场景。

两批原始运行均为 `passed`，所选场景各三轮通过；运行期间源码与采样器未变化。完整本地归档位于 `test-lab/results/<runId>/summary.json`，不随仓库分发。本次 README 更新仅整理已有数据，没有重新执行性能测试。

采样定义与运行方法见[原生测试工具](../test-lab/README.md)及 [opening.mjs](../test-lab/runner/opening.mjs)。固定测试语料需按测试工具说明单独准备。
