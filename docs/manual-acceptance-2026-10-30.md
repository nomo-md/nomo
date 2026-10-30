# Nomo 桌面模拟验收记录

日期：2026-10-30，Asia/Shanghai。

本轮通过真实 Windows 窗口的键鼠操作测试当前 Nomo，已确认 **3 项可复现问题：查找框粘贴崩溃、双栏阅读位置不同步、PDF／Word 遗漏含图片的表格**。未修改产品源码或修复这些问题，未提交、未推送。

范围取自“对齐新增功能与原版”（01a187e4-a57d-7943-a47c-b7f3522c1796）的五阶段工作，以及“检查节点与原版差异”（01a1869b-c168-7552-be8c-7edfee862a89）的节点修复。两段聊天已实际读取；没有运行旧 WebView 版做逐帧 A/B。桌面操作由 Terra 单独执行，主代理核对崩溃日志、磁盘内容、导出结构与构建输入。

## 构建与环境

| 项目 | 实测记录 |
| --- | --- |
| 构建 | Nomo 0.6.2，默认 Release；cargo build --locked --release --bin nomo 通过，99 项既有警告 |
| 代码 | 分支 gpui-editor；HEAD 09f9e8fda8e2475e3ad816e948a7950e3f5d4c9c，含开始前已有的未提交改动 |
| 二进制 | C:\Users\since\Documents\nomo\target\release\nomo.exe |
| SHA-256 | A536E1944F78FADFBCCB46ED13220ABF51205D8A5F9329EF4467F87BCFFCAA51 |
| Windows / GPUI | Windows 11 Pro for Workstations 10.0.26200 / GPUI 0.3.10 |
| 初始窗口 / DPI | 应用逻辑视口 1280×820，外框截图 1282×821；open-profile 的 scale_factor=1.0 |
| 隔离目录 | C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01；NOMO_DATA_DIR 指向其 data 子目录 |
| 资源 | 可丢弃 Markdown、400×200 PNG、三帧 GIF、SVG、中文公式、Mermaid、120 行表格、约 2.33 MB / 4500 节长文档 |
| 构建输入复核 | 364 个文件；与构建后记录相比变更 0 个 |

截图由 Computer Use 在执行中内联显示，没有另存截图文件或录制视频。以下“通过”只适用于明确列出的步骤，不代表 72 项验收清单全部通过。

测试材料修正：最初公式样本的反斜杠被准备脚本错误转成 TAB／换页字符；经字节核对修正并重载后中文公式显示正常。这是测试数据问题，已排除出产品缺陷；所有导出内容核对使用修正后的样本。

## UI-001 / P1：点击空查找框后粘贴导致整个应用退出

- 资源：fixtures/03-search.md。
- 步骤：打开文件 → Ctrl+F → 点击空查找框中占位文字右侧（1280×820、100% DPI 下窗口坐标约 698,148）→ 粘贴 apple 或单个 a。
- 预期：输入查询并显示匹配结果。
- 实际：Nomo 窗口消失，进程退出。两次独立启动均复现（PID 23752、18392）。
- 证据：nomo-stderr.log、nomo-run02-stderr.log；第二次启用完整调用栈，定位 TextField::on_paste → String::replace_range，报错 range end index 6 out of range for slice of length 0。
- 代码核查：[text_field.rs:217](C:/Users/since/Documents/nomo/src-gpui/src/ui/text_field.rs:217) 从占位文字布局获取点击偏移，真实 content 为空；:136 直接用该偏移替换。占位文字“查找”的 UTF-8 长度为 6，与错误一致。
- 建议：空内容命中位置固定为 0，写入前保证选区属于真实内容并符合字符边界；同时核对共用该控件的替换、设置与搜索入口。
- 验证边界：桌面工具 type_text 在此次调用栈中走粘贴路径，不能据此声称已覆盖实体键盘逐字输入和 IME。其他输入框受影响是代码推断，未逐个实测。

## UI-003 / P1：PDF／Word 导出遗漏含 HTML 图片的整张 Markdown 表格

- 资源：fixtures/05-export.md；最小复现 fixtures/09-export-table-repro.md。
- 步骤：打开最小文档，确认界面显示两张表格，第一张含 img 图片及 CELL-MUST-SURVIVE-9347 文本 → 文件菜单导出 PDF → 文件菜单导出 Word。
- 预期：两张表、图片及所有单元格内容保留。
- 实际：两个格式都只保留普通对照表；图片表的表头、图片和文本标记一起消失。前后段落及末尾标记仍在，原文未丢失。
- 复现：综合样本、最小样本各一次；每次均覆盖 PDF 和 DOCX。对应 HTML 综合导出保留了图片表。
- 证据：exports/09-export-table-repro.pdf、exports/09-export-table-repro.docx、export-validation.json。最小 PDF 只有 94 字符提取文本、0 张图片；DOCX 仅 1 张 2 行对照表、0 张图片，XML 关系合法但没有 CELL-MUST-SURVIVE-9347。
- 排除：并非整表转为图片或仅文字提取器漏字，DOCX 的所有 w:t 与媒体资源也缺失。不能将“文件能解析”计为导出正确。
- 代码核查范围：[model.rs:156](C:/Users/since/Documents/nomo/src-gpui/src/document_export/model.rs:156) 的 skip 过滤与 :266 的 HTML 行内原子跳过共用同一路径；尚未修改或用定向单测确认具体根因。
- 建议：核对 HTML 行内图片后的 TableCell／Table 结束事件是否被 skip 过滤；保证节点范围过滤不丢弃容器结束事件，再复验同一最小文件。
- UI 完成提示：桌面快照未捕捉到警告；不能据此断言产品绝无瞬时提示。

## UI-002 / P2：切到双栏后新出现的一栏回到文档顶部

- 资源：fixtures/02-media.md，图片自然尺寸章节。
- 步骤 A：语义模式滚到图片 → 切双栏。步骤 B：源码模式滚到同一位置 → 切双栏。
- 预期：新栏围绕当前阅读位置展开，两个视图的内容相互对应。
- 实际：原栏保留图片位置，新栏显示文档顶部；两个方向均观察到。
- 影响：长文档模式切换后需要重新寻找对应位置。尚无正文丢失证据。
- 证据：Terra 的两次桌面操作与内联截图；Computer Use 的截图保存限制下未落盘截图。
- 建议：初始化新 Pane 时应用当前语义阅读锚点，并在异步布局完成后校验版本与位置。

## 待复核体验现象

这些现象按用户要求保留，但不并入上述三项确认缺陷。

1. 撤销后视口上移：正文输入 TEST 撤销时观察到约 48px 上移，表格输入 XYZ 撤销时约 220px。尚未严格固定编辑前锚点与异步提交完成时机，无法排除恢复历史阅读位置等合理行为。
2. 立即截图短暂保留旧状态：首次正文 Ctrl+Z 后的快照仍见 TEST，随后一次操作与刷新后消失。缺少独立等待对照和帧时间证据，不能据此确认需要两次撤销，亦不能量化卡顿。
3. 主题切换中捕获半透明过渡帧：后续稳定帧正常。目前按过渡观察记录，不判定为异常闪烁。
4. 撤销分隔线时大纲瞬态报错：在 10-new-save.md 空段按 Ctrl+Shift+H 插线、Ctrl+Z 撤销，立即帧右侧大纲出现红字“宿主提交失败：文档 revision 已变化：expected=2, actual=3”及“重试”入口，下一帧恢复。仅观察到一次；后续尝试未确认编辑焦点，没有成功再次插线，因此不算有效复现。

未进行连续录屏或帧时间采样；静态截图和工具调用耗时不能证明不存在闪烁，也不能直接作为应用延迟测量。


## 实际操作与覆盖边界

- 查找：Ctrl+F 打开、Ctrl+H 展开替换、再次 Ctrl+H 关闭。03-search 空查找框点击 x698,y148 后粘贴 apple 或 a，进程退出两次；主代理日志确认 on_paste 越界 panic。type_text 为粘贴，不等同键盘输入。未点击框直接粘贴 apple 后计数 0/3，Enter 定位 1/3；替换框按 Home 后粘贴 pear，全部替换后正文三处 pear。
- 新建：Ctrl+N，粘贴 `Acceptance new document 保存重开 20261030`，保存为 fixtures/10-new-save.md，关闭并双击重开，内容显示完整；主代理确认磁盘内容。粘贴汉字不是 IME。
- 表格：01 单元格粘贴 XYZ 显示，Ctrl+Z 稳定后恢复；表头前插入空行后撤销恢复，末行后插行并撤销。尚未验证列操作/删除。
- Details：标题中间 Enter 未见内容变化；菜单取消折叠块解包为标题及两段正文，Ctrl+Z 恢复卡片。边界 Backspace 未测。
- Callout：NOTE 正文末 Enter 新建块内空段，再 Enter 光标退出到块外，原内容保留。空块删除未测。
- 06 块尾：Ctrl+End 到块后，粘贴 AFTER-END、Ctrl+Z 后消失；代码中粘贴 XYZ 并撤销；语言 Rust 选择器筛选 Python、回车应用、撤销恢复 Rust。保存后主代理确认与 baseline 字节一致。02 代码复制按钮出现勾，剪贴板内容未核实。
- 媒体：Rust 多行高亮、Mermaid 可见；修正 fixture 后中文公式分式、上下标及行内中文可见；400×200 PNG 截图约自然尺寸、SVG 可见；GIF 连续截图从 FRAME 2 到 FRAME 1，动画推进，未测帧率。
- 模式：图片位置从语义切双栏，新增源码栏回顶部；从源码切双栏，新增语义栏回顶部，原栏保留位置，两向复现。单栏切换和双栏显示可用，但新栏阅读位置不同步。
- 04 大文档约 2.33 MB：滚轮从第1节至第5-6节；Ctrl+End 到 END-LARGE-4500，Ctrl+Home 回第1节。非性能测试。
- 外观：四内置主题卡均可选，深色/浅色稳定帧呈对应颜色；GitHub 导出 exports/github-theme.json，重新导入新增 GitHub（自定义）并选中；主代理确认 JSON 和导入文件存在。截图偶见过渡中半透明帧，不能据此认定闪烁。
- 导出：05 HTML/PDF/DOCX 已保存 exports，主代理解析 12 页 PDF、长表首末行及尾标记，DOCX关系完整；阅读器视觉排版未测。图片表 UI 可见，但 PDF 和 DOCX 丢失整张表。09 最小复现 UI 显示图片及右格 CELL-MUST-SURVIVE-9347；导出 PDF/DOCX 后主代理解析均无该表及标记，普通对照表和前后标记保留。
- 外部修改：07 干净文档由磁盘 V1 改 V2 后自动显示 V2；有本地未保存 V2+LOCAL-UNSAVED-V3 时外部改 V3，弹外部文件变更提示，列出重新载入/另存为/覆盖外部/忽略，提示自动保存暂停，背景保留本地内容；选择忽略后本地草稿及未保存标记保留。收尾时另存为成功保留草稿及原磁盘 V3；覆盖、重新载入分支未测。
- 撤销时部分截图视口有上移，因编辑前位置及异步刷新尚未做严格对照，不作为已确认缺陷。自动化等待时间不作为卡顿证据。


## 后续补充和最终覆盖状态

- 应用日志 scale_factor=1.0，逻辑视口1280×820，截图外框1282×821，Windows缩放100%。
- Ctrl+F 重复关闭纯查找。10空段 Ctrl+Shift+H 插入水平线、没有跳主页，Ctrl+Z 稳定后线消失；撤销即时一帧大纲红字“宿主提交失败：文档 revision 已变化：expected=2, actual=3”，显示“重试/大纲暂不可用”，下一帧自动恢复。第二次尝试因焦点不确定且线未插入，不能算有效复现；单次观察，未确认缺陷。
- 01 Front Matter 标题追加 -EDIT，确认后卡片更新并保存；主代理 diff 确认仅标题和 Callout 空行变化，custom 未知字段保留。
- 块公式草稿改 x^2+1，确认后显示 x²+1，Ctrl+Z 稳定后恢复原中文分式。Mermaid 时序图草稿追加 X 后取消，原图保留。
- 高级设置将新建快捷键改 Ctrl+O，提示与打开文件冲突，Enter 后拒绝，Escape 恢复 Ctrl+N。默认图表从流程图改时序图，段落→图表→默认图表插入后显示 User/App 时序图。
- Ctrl+Tab 20次，每次获取窗口标题，10个标签顺序循环两遍。Ctrl+E 20次，每次获取标题，最终源码模式且进程存活；未逐次截图确认模式，不能称20次有效切换或双栏20轮。另有语义→源码→双栏实际可见。误点模式批次改变宽度，不能计入有效轮次。
- 窄窗：窗口拖动及系统菜单大小操作未形成有意义窄窗，未验证。中文 IME：主代理确认中文键盘已配置；Shift 后逐键 n、i 未观察候选，未验证组合与提交。禁止 Win 键工具约束下未用 Win+Space。
- 03 全部替换后保存。07 的本地草稿已另存归档，原磁盘 V3 保留。截图仅内联，无持续视频或帧率数据。

最终未覆盖：窄窗布局、真实中文IME候选与提交、双栏20轮、标签选择与阅读位置全生命周期、表格列和删除、Details边界Backspace、空Callout删除、块注释编辑、行内公式编辑、图表确认提交、代码复制内容与长代码滚动、复杂HTML/脚注/TOC完整编辑、导出阅读器视觉排版、跨平台和完整性能验收。

## 最终退出状态

冲突草稿通过另存为保存至 fixtures/07-local-draft-preserved.md，磁盘核对包含 LOCAL-UNSAVED-V3；原 fixtures/07-external.md 仍为 EXTERNAL-DISK-V3-CONFLICT。关闭窗口时另一个未命名图表草稿保存至 fixtures/11-diagram-draft-preserved.md。随后主窗口关闭，PID 24796 不再存在。未点击放弃修改。

## 独立核对与性能观察

- 06-end-block.md 保存后与初始 45 字节基线完全一致：代码输入、语言切换、块后输入及自动补入的换行均未残留。
- 01-editor.md 的持久差异仅为 Front Matter 标题追加 -EDIT 与 NOTE 退出产生的空行；custom 未知字段保留。
- 10-new-save.md 初次新建、保存、重开时与输入文本一致；后续 IME 尝试实际产生了拉丁字母 ni，作为测试输入保留，不能视为中文输入法通过。
- 05 的 HTML、PDF、DOCX 文件可解析；中文文本、120 行长表首尾和文档末尾标记存在。PDF 为 12 页，长表所在第 7–11 页重复表头。此结果不覆盖阅读器视觉排版，且不能抵消 UI-003 的内容遗漏。
- 09 的最小 PDF 为 1 页且没有图片；DOCX 仅有 1 张 2 行普通对照表、没有媒体资源，图片表的 CELL-MUST-SURVIVE-9347 在两个格式中均不存在。
- 冲突草稿另存为 07-local-draft-preserved.md 后仍含 LOCAL-UNSAVED-V3；原 07-external.md 保持 EXTERNAL-DISK-V3-CONFLICT。图表草稿也已另存，主测试进程正常通过界面关闭；没有丢弃未保存草稿。
- 大文档内置采样：正文 2,331,046 字节，首窗口 10,909 字节、当时已索引 65,299 字节，首帧字段约 109.9 ms。这是一次首窗记录，不是完整打开耗时、持续滚动帧率或三轮性能基准。
- 主测试进程 PID 24796 的 2 秒间隔采样：总工作集峰值 327.62 MiB；私有提交峰值 281.16 MiB。这两个指标不相加；未测私有驻留、GPU、输入帧时间。混合场景单轮数据不能证明泄漏、无泄漏或给出永久内存上限。

## 可核查材料

- [问题、最小复现和待复核体验现象](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/confirmed-issues.md)
- [桌面操作原始记录](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/ui-observations.md)
- [查找崩溃完整调用栈](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/nomo-run02-stderr.log)
- [导出遗漏最小 Markdown](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/fixtures/09-export-table-repro.md)；[对应 PDF](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/exports/09-export-table-repro.pdf)；[对应 Word](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/exports/09-export-table-repro.docx)
- [导出独立解析结果](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/export-validation.json)；[元数据保存差异](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/01-editor-saved.diff)；[块末尾撤销字节核对](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/end-block-undo-validation.json)
- [构建环境](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/environment.json)；[Release 构建日志](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/build-release.log)；[构建输入核对](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/source-verification.json)
- [进程采样](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/process-samples.csv)；[应用内首窗采样](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/open-profile.jsonl)；[采样与最终文件状态汇总](C:/Users/since/Documents/nomo/.artifacts/manual-acceptance-20261030-01/run-summary.json)

本轮仅使用隔离配置和临时文档。没有进行安装、系统注册、网络上传、发布或远端写入。用户要求在今后的相关真人模拟测试中记录不合理交互、卡顿、异常刷新、闪烁和不流畅，这项偏好已另行保存。
