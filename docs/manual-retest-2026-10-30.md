# 2026-10-30 修复后的真人模拟复测

本轮先修复上轮三个问题，再通过鼠标、键盘及系统文件对话框操控 Release 应用，三个原始场景均通过复测。扩展操作另记录 1 项已确认导航缺陷、2 项阅读位置体验观察，尚未修复。测试已结束，隔离实例正常退出；原始截图保留在桌面操作工具历史中，文件保存文字观察和可独立检查的导出产物。

## 构建与环境

- Nomo 0.6.2，GPUI 0.3.10，Windows 11 10.0.26200，分支 `gpui-editor`，基线提交 `09f9e8fda8e2475e3ad816e948a7950e3f5d4c9c`，包含工作区已有改动。
- 最终 Release SHA-256：`5F65E11A8F7F1DBCB8F552BA53604229C1A042D1C77257CA7090E18774FA9F7B`，67,317,760 字节；测试 PID 12788。
- 浅色，截图窗口 1282×821；内容宽度 60%，应用缩放按隔离配置默认 100%。本轮未直接读取 DPI。
- 独立数据目录和测试副本位于 [任务目录](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01)。未操控另一个 package-work 应用实例。

## 原问题与修正

| 编号 | 修正 | 本轮状态 |
| --- | --- | --- |
| UI-001 / P1 | 空查找框不再用占位文字或过期布局生成光标位置；空框字符位置为 0 | 原崩溃路径 3 轮通过；中文与 Emoji 粘贴、空框删除、英文单键、替换及撤销通过。保存后样本与 baseline 的 SHA-256 一致 |
| UI-002 / P2 | 新栏使用阅读锚点所在的同修订片段，首次布局前设置位置；索引刷新保留顶边距 | 中部、图片区域两向各一次通过，即时及稳定截图均在对应章节。大文档切换保持文末附近，但尾标记离开视口，另行记录 |
| UI-003 / P1 | 导出跳过图片/HTML 源码时，保留已经打开的外围容器结束事件；共同修正 HTML 和 PDF/Word 路径 | 最小及综合样本分别经 UI 导出三种格式；独立解析全部通过 |

UI-002 的补诊断修正了上轮观察的精度：本轮修复前捕获的是“新增源码栏先显示顶部，下一次截图恢复到中部”，未复现持续停留顶部。修复后的同一路径即时截图不再出现顶部。截图不足以测量持续时间或证明完全没有闪烁。

上述修改只提前布局当前有界片段，复用既有缓存、请求生命周期和同一文档历史；未增加全文副本、常驻任务、依赖或额外缓存层。

## 导出产物复核

| 样本 | Word | PDF | HTML |
| --- | --- | --- | --- |
| 09 最小复现 | 2 表、4 行、1 图，所有关系有效 | 1 页、1 次图片绘制 | 2 表、1 图，表格标签完整闭合 |
| 05 综合样本 | 3 表、127 行、8 图，所有关系有效 | 12 页、8 次图片绘制；长表跨页重复表头 | 3 表、8 图，表格标签完整闭合 |

两组产物均保留图片表文字、前后正文及尾标记；综合样本同时保留 `EXPORT-ROW-001`、`EXPORT-ROW-120`。PDF 复用图片资源，综合样本按页统计的独立图片资源为 7，实际绘制 8 次，不能将资源复用误判为缺图。本轮检查文本和文件结构，未在 Word/PDF 阅读器中验收视觉分页。

[核验 JSON](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/export-validation.json) · [导出产物](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/exports)

## 扩展桌面回归

- 表格单元格追加 `-TEST` 后撤销并保存；该步骤完成时 `01-editor.md` 与原样本 SHA-256 一致。随后 Callout 正文末连按两次 Enter，光标退出块外且内容保留；最终保存文件仅比 baseline 增加 NOTE 块内外空行。
- 块公式改为 `x^2+1`，确认后显示新公式，撤销后恢复中文分式；代码原位输入 `TEST` 后撤销；Mermaid 草稿取消后恢复原图。保存后的 `02-media.md` 与原样本 SHA-256 一致。
- 新建 `10-retest-new.md`，保存、关闭、重开后 UI 内容完整，磁盘核对正文为 `RETEST-NEW-20261030 中文保存`。随后新增空段，Ctrl+Shift+H 插入分隔线，Ctrl+Z 后横线消失并保存空段；本次截图未捕获上轮的短暂大纲 revision 红字，仅显示当前文档没有标题，不能据一次未捕获关闭旧观察。
- Ctrl+Tab 共 20 次，每次读取窗口标题，均切到预期下一标签；未逐次检查正文位置或连续画面，只证明切换目标正确。

## 自动检查

- 默认及 `nomo-mark-demo` 的 `cargo check --locked --bin nomo` 通过；最终 `cargo build --locked --release --bin nomo` 通过。
- 既有 `document_export::tests` 7 项、`nomo_mark::viewport_model::tests` 3 项通过。未新增或修改产品测试用例。
- 默认/开发入口仍分别有 99/96 项既有编译警告，未顺手处理。
- 最终差异检查通过；构建后的 328 个输入文件 SHA-256 无变化，测试二进制与上述哈希一致。运行 stderr 为空，测试 PID 12788 及采样 PID 28048 均已退出。未提交或推送。

## 持续观察与未覆盖范围

本轮新增记录，尚未修改代码：

- **UI-004 / P2：新增源码栏的 Ctrl+Home 只到当前片段起点。** 重开 `04-large.md`，单语义聚焦后 Ctrl+End 到 `END-LARGE-4500`，切双栏，点击新增右源码栏 4499 节正文并确认光标，再 Ctrl+Home。右栏显示 `KEEP-4480`/4481，而非 0001；左栏仍在 4499/4500。有效复现 1 次。此前未重新确认焦点的两次尝试不计入次数。代码核对：[新增控件初始化](C:/Users/since/Documents/nomo/src-gpui/src/nomo_mark/window_panes.rs:311) 未继承 `windowed_navigation`，[默认值](C:/Users/since/Documents/nomo/src-gpui/src/nomo_mark/view.rs:167) 为 false，[文档边界动作](C:/Users/since/Documents/nomo/src-gpui/src/nomo_mark/keyboard.rs:8) 因而只移动到片段边缘。建议继承宿主的窗口导航标志，再覆盖 Home/End 和跨片段翻页。
- **UI-005 / 阅读位置体验观察：文末切双栏后尾标记离开视口。** 单语义 Ctrl+End 后尾标记可见，切双栏后仍在 4497–4500 节附近，但尾标记不再可见，2/2 次。切回单语义后可见；这不是回到文首。可能涉及缩窄后的换行和中部阅读锚点策略，尚未认定为本次修改引入的回归。
- **UI-006 / 待严格对照的观察：对象撤销后视口上移。** 公式撤销前可见中文公式/Mermaid，稳定后显示长代码/中文公式；代码撤销前为长代码/中文公式，稳定后显示多行高亮/长代码。内容已恢复且保存文件与 baseline 一致，但没有事先固定阅读位置和光标，暂不认定为缺陷或不流畅的量化结论。

未覆盖：Details 完整边界/解包（标题末 Enter 无明显画面变化，无法可靠确认语义）、真实中文 IME 候选提交、大文档真正中部的长距离滚动、连续录像/FPS、导出阅读器视觉分页、其他主题/DPI、跨平台及三轮性能对照。静态截图无法证明完全无闪烁或量化延迟，自动化工具等待时间也不作为卡顿证据。

本次混合流程采样时间为 11:47:58–12:39:40（UTC+8），共 1,543 条：工作集峰值 254.88 MiB，私有提交峰值 202.98 MiB，分别统计、不相加；未采 GPU 指标。这不是三轮性能基准，不能据此认定泄漏或固定内存上限。[进程采样](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/process-samples.csv) · [最终核对 JSON](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/final-verification.json)

所有本轮修改过的测试文档已保存，未点击放弃修改。最终 `02-media.md`、`03-search.md`、`04-large.md`、`05-export.md`、`09-export-table-repro.md` 与 baseline 一致；`01-editor.md` 保留 Callout 测试空行，`10-retest-new.md` 保留新建正文和分隔线撤销后的空段。

原始记录：[桌面观察](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/ui-retest-observations.md)、[双栏诊断](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01/diagnostic-ui.md)、[构建与测试日志](C:/Users/since/Documents/nomo/.artifacts/manual-repair-retest-20261030-01)。
