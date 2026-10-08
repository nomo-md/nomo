# KP 精细排版：实现与验收记录

本记录对应 2026-10-08 批准的实施计划。代码已接入主链路，**尚未达到 P0–P5 全部验收完成**。

## 启用与发布关卡

偏好设置 → 编辑器 → 段落排版，只保留“精细排版”总开关。开启后使用默认简体规则，标点压缩、行末悬挂和内容所需的行距调整一并生效。原有 `typographyProfile`、`typographyHanging` 用户配置不再读取；内核仍保留规则档案与悬挂参数供对照测试使用。

开发版默认关闭。`src/lib/typography/types.ts` 的 `DEFAULT_TYPOGRAPHY.enabled` 和 Quick Look 原生配置缺省值保留发布关卡；完成下述原生验收后改为默认开启。用户明确关闭的持久设置继续有效。未写入 Markdown 元数据。

## 代码边界

| 部分 | 入口与职责 |
|---|---|
| 纯计算 | `src/lib/typography/knuthPlass.ts`：候选行计算、松紧类别、连续断词、短尾行、强制断点、搜索预算。与 DOM 无关。 |
| 语言规则 | `rules.ts`：Unicode 断行机会、简繁标点策略、字素边界、中西文视觉空白、显式软连字符。 |
| 测量 | `measure.ts`：真实字体连续前缀测量，字框空白测量，公式/图片/行内代码尺寸与基线。 |
| 渲染与坐标 | `renderPlan.ts`、`positions.ts`：视觉行、间距、边缘调整、原文位置和视觉位置的边界倾向。编辑器继续使用 ProseMirror 原生 DOM 坐标映射。 |
| 调度 | `solver.ts`、`typography.worker.ts`：内联 Worker、请求编号、错误/超时回退。主线程测量，Worker 求解。 |
| 编辑态 | `editor-core/plugins/typography.ts`：约 150 ms 空闲调度、组合输入与拖动选区保护、段落缓存、过期结果丢弃、Decorations。`TypographyBreakNodeView` 只负责源码换行的视觉投影。 |
| 只读 | `dom.ts`：预览、Quick Look、导出共用；保留语义副本，宽度/字体变化后重排，关闭时恢复。 |
| 导出 | `ProseMirrorEditorCore.getExportHtml()` 从文档序列化，保留必要的图表/代码渲染快照，重新生成公式。导出清理装饰，再嵌入运行时、Worker 和 KaTeX 字体。 |
| PDF | Windows 与 macOS 按纸张方向和左右边距计算版心，调用 `__NOMO_PREPARE_PRINT__(widthMm)`，等待字体、图片及最终排版。就绪失败或超时不能打印为成功。 |

普通单换行保留在文档中。精细模式将其投影为上下文所需的空白；硬换行仍为强制断点。编辑态复制与保存继续从语义文档序列化，不复制视觉断点。排版事务不改变 dirty、正文修订或撤销历史；会使滚动同步几何缓存失效。

`linebreak@1.1.0` 提供 Unicode 13 的 UAX #14 数据；`Intl.Segmenter` 提供运行环境的字素边界。因此不能宣称最新 Unicode 版本的完全一致性，也不以复现 TeX 输出作为验收标准。其他文字、复杂 HTML、无法容纳的不可拆对象及预算超限保留原生段落布局。

## 初始预算与已知工程边界

- 测量约每 8 ms 让出主线程；单段超过 20,000 个 UTF-16 单元或约 1,500 ms 测量预算时回退。
- 搜索默认最多 100,000 个候选、约 100 ms；检查预算时整段回退，不返回半段文本。
- Worker 请求 5 s 超时；导出资产等待 20 s、排版稳定等待 30 s。失败不记录正文。
- 未改动段落复用已发布装饰；字体或几何失效时重新测量。可见段落优先计算，按约 16 ms 的发布间隔提交完整段落，未处理段落暂时保留原布局；中途取消的未发布结果不会进入缓存。
- 简繁档案目前提供基础禁则、成对省略号/破折号、连续标点与边缘压缩。大量不同字体、繁体标点风格和高行内对象仍需视觉样例扩充。
- 只读适配器覆盖段落、直接含行内内容的列表项和单元格；混有嵌套块的复杂 HTML 使用现有结构，不强行改写。表格列或 HTML 指定居中、右对齐时保留原生对齐布局。
- 原文不被改写已经有测试；原生输入法、跨视觉行光标/选区行为不能由 jsdom 或合成 composition 事件代替。

## 本次验证

针对性回归入口：

启动 `pnpm dev` 后打开 `/scripts/fixtures/kp-typography.html` 可重放浏览器原型检查。该独立测试页明确开启精细模式，不改变发布默认值；组合事件按钮仅模拟生命周期，不等于真实中文输入法验收。

```powershell
pnpm exec vitest run src/lib/typography src/lib/editor-core/typography.test.ts src/app/services/exportService.test.ts src/app/services/settings.test.ts src/quicklook/preview.test.ts src/lib/editor-core/createEditorCore.test.ts src/lib/editor-core/markdown.test.ts src/lib/editor-core/clipboardMarkdown.test.ts
pnpm build
pnpm run build:quicklook-renderer
cargo check --manifest-path src-tauri/Cargo.toml
pnpm check
```

- KP 测试使用 100 组小规模穷举对照；额外覆盖禁止/强制断点、零伸缩、无解、预算、悬挂、行高、边界映射、NBSP、组合字符、emoji、软连字符。
- 编辑器与导出回归覆盖开关不改变原文/dirty/历史、语义导出保留软换行、清理装饰时保留公式、自包含脚本与字体。
- 显式软连字符采用实际字体宽度，段落末尾不额外显示连字符；编辑与只读采用相同的文本 shaping 配置。
- 针对性测试共 220 项通过、2 项既有跳过项。额外覆盖源码视图尚未同步时导出当前正文，避免导出陈旧的语义视图。
- Chromium 实际页面检查已观察到：混排/粗体/链接/公式段落完成排版，输入后撤销恢复源码，关闭恢复原生换行，离线运行时按 170 mm（约 642.52 CSS px）版心重排且无外部脚本。
- 主应用、Quick Look 前端构建和 Windows Rust 检查通过。构建保留原有大 chunk 提示与 Rust dead_code 提示。
- 百段混排样例在本机 Chromium、约 226 CSS px 段落宽度下，100/100 段完成、0 回退、源码一致，单次约 3,768 ms。修正测试夹具缺少 `min-width: 0` 之前曾耗时 24,147 ms；该夹具会随内容改变 flex 最小宽度，不能将其与固定宽度结果混用。主应用的 `.semantic-pane` 已有该约束。这些是开发检查数据，不是跨平台性能承诺。
- 全量 `pnpm check` 的既有阻碍是 `plugins/pendingInlineMark.test.ts:763,798` 的 MouseEvent/PointerEvent 类型不匹配；没有为本任务改写该无关测试。

## 未完成验收及下一步

1. **P0/P4 原生编辑关卡**：Windows WebView2 + 微软拼音、macOS WKWebView + 中文输入法。覆盖组合/候选提交、光标两侧倾向、跨视觉行删除/选区、链接/右键、搜索、复制粘贴、撤销重做及双栏同步。当前 Chromium 检查不替代这项验收。
2. **P2 视觉矩阵**：增加简繁、不同系统字体、字号/粗斜体、图片/公式/脚注、窄单元格、连续空白及悬挂几何的逐项验收；常规支持场景不能靠原生回退过关。
3. **P3 原生导出**：实际生成 Windows/macOS PDF，验证 A4/Letter、横竖方向、页边距、字体延迟、图片失败和超时失败；macOS Swift/Rust 编译与 Quick Look 宿主验证仍需 macOS 设备。
4. **P5 性能与发布**：记录长文档、长段落、快速缩放、频繁输入、异步过期与取消的实测数据，然后才解除默认启用关卡。

这些项目完成前，不应将当前状态描述为完整交付或两平台验收通过。
