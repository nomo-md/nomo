# 2026年11月6日 Nomo 完整文章逐键测试

> 2026-11-07 更新：BUG 01（空 NOTE 续写）与 BUG 02（下划线标签错序）已修复，并完成最终 Release 版本的逐键复验、撤销重做和保存重开；详见文末追加记录。

本轮已完成逐键写作、后续交互、保存重开和隔离实例退出，并保存了测试文章与证据。主文章从空白开始，以花园为主题覆盖 33 类节点；另一个仅含 `abc` 的辅助文档只用于新建焦点与标签隔离检查。

33 类节点的完整子范围判定为 **7 通过、22 部分、4 失败**；45 项基础操作为 **24 通过、20 部分、1 失败**。四个失败节点行涉及三项问题，其中实体与下划线问题跨两类节点。前两项各有两条有效异常路径；文末链接问题前两次出现，后续稳定等待对照未复现。**本轮完成不等于所有节点、交互或中间态全部通过**，具体限制逐行列出。产品代码未修改，问题未修复，未提交或推送。

## 环境与范围

| 项目 | 本轮固定值 |
| --- | --- |
| 系统 | Windows 11 10.0.26200，PowerShell 7.6.9 |
| 构建至退出核对时段 | 2026-11-06 17:33–20:39，Asia/Shanghai；包含操作、等待与取证 |
| 产品 | Nomo 0.6.2，默认 Workspace 入口 |
| 实际桌面环境 | 窗口截图 1282×821，浅色主题，工具栏显示缩放 60%；DPI 未取得 |
| 输入法与模式 | Microsoft Pinyin；语义、源码、双栏均使用；真实 IME 仅覆盖普通段落的提交与取消 |
| 分支及提交 | `gpui-editor`，`dbce8b8f0034edff2afa98ca92df84cd8cf4688f`，包含开始前已有未提交改动 |
| 构建 | `cargo build --locked --release --bin nomo`，退出码 0；98 项既有警告 |
| 固定二进制 | `.artifacts/full-article-test-20261106/bin/nomo.exe`，70,052,352 字节 |
| SHA-256 | `B74FAE2E977D5DFCE104313989C6371659007EDD5FC68A9A449675507C3F5090` |
| 构建输入 | 448 个文件，构建期间哈希均未变化 |
| 隔离实例 | PID 13500，独立 `NOMO_DATA_DIR`，stdout/stderr 写入本轮证据目录 |
| 主文章 | [article.md](../.artifacts/full-article-test-20261106/documents/article.md) |
| 非正文资源 | 同目录 `img.png`，由仓库已有图片复制；正文未预写 |

构建清单：[build.json](../.artifacts/full-article-test-20261106/evidence/build.json)；会话记录：[session.json](../.artifacts/full-article-test-20261106/evidence/session.json)；开始前状态：[git-before.txt](../.artifacts/full-article-test-20261106/evidence/git-before.txt)。本轮不修改产品代码，不提交或推送。

范围为[节点目录](../.artifacts/full-article-test-20261106/evidence/node-catalog.json)中的 33 类节点与文字结构，包括 H1 至 H6、5 类提示块、7 类 Mermaid、安全 HTML 和合并表格。它是本轮覆盖边界，不表示穷尽全部嵌套组合、样式属性或应用功能。

## 输入与判定方法

执行[真人写作测试方法](manual-writing-test-method.md)，并逐行报告[基础写作清单](basic-writing-checklist.md)的 45 项覆盖情况。语法符号分别按键、观察未闭合状态；正文使用逐字符按键。中文以真实 IME 候选、提交和取消取证。剪贴板操作单独记载。

桌面仅由 Terra 操作本轮隔离窗口。主代理核对当前源码职责、构建、保存文件及日志。截图证据位于桌面工具历史，不能把未落盘的截图列为本地文件。保存字节核对与 UI 判定分别记录；源码标记存在不等于渲染、光标和交互通过。

UI 插入表格、Details 的结构以及应用派生内容，不算手写源符号的证据；对应类型另有逐键源码样本。格式快捷键和后续剪贴板用例也作为独立编辑路径记录。

早期标题和第一句正文为逐字符输入，但在整句后观察，没有达到短词后的观察粒度；这项限制保留在结果中。逐键 JSONL 从 N18 阶段开始；更早动作使用工具历史和分组操作记录，不补造逐键日志。

后半段部分节点是在同一调用中逐键发送、每键收集截图，调用结束后才统一查看，**不满足“检查当前符号后再输入下一符号”的严格观察规则**。明确涉及晚期 font/div、波浪号重测、锚点/自动/引用式链接、带空格管道表；七种图表和部分 HTML 也有此类批次。其真实逐键输入、保存和最终渲染证据仍保留，但不能据此宣称所有中间态都经过实时审视。后续单步补测与原路径分别记录，不回改原日志。

表情节点有一次系统 `Super+period` 选择器操作，与 [computer-use 技能](C:/Users/since/.codex/plugins/cache/openai-bundled/computer-use/26.1103.31037/skills/computer-use/SKILL.md)要求遵守的 [guidance.md](C:/Users/since/.codex/plugins/cache/openai-bundled/computer-use/26.1103.31037/docs/guidance.md) 规则冲突：`Do not use the Windows key or shortcuts involving the Windows key.` 该路径已停止，不计入合规键盘覆盖；原 Unicode 表情也已通过 UI 删除，[77](../.artifacts/full-article-test-20261106/checkpoints/77-picker-emoji-removed.md) 仅有这一处差异。另逐键输入的 `&#128522;` 及其表情渲染保留，二者不混算。实体输入也不等于原始 Unicode 或 IME 输入。

## 节点结果

共 33 类：通过 7 项、失败 4 项、部分 22 项。状态覆盖每行列出的子范围；“部分”不等于未输入，也不等于完整验收通过。

| ID | 节点 | 状态 | 实际覆盖及边界 | 保存证据 |
| --- | --- | --- | --- | --- |
| N01 | Front Matter | 部分 | 文首 YAML 逐键形成并显示卡片；单独字段修改未测。 | [01](../.artifacts/full-article-test-20261106/checkpoints/01-front-matter.md) |
| N02 | 正文 TOC | 部分 | 手写标记后H1–H6仍多次显示暂无标题；75仍空，76后续阶段派生六条目录，具体触发未隔离。最终卡片六条可见，点其Soil链接滚到标题且光标在开头。 | [15](../.artifacts/full-article-test-20261106/checkpoints/15-toc.md)、[17](../.artifacts/full-article-test-20261106/checkpoints/17-six-headings.md)、[75](../.artifacts/full-article-test-20261106/checkpoints/75-ui-table.md)、[76](../.artifacts/full-article-test-20261106/checkpoints/76-table-outside.md) |
| N03 | 标题 | 部分 | H1–H6均逐键写入；Morning开头Enter/Undo、改级别/Undo及H1/H2/H3大纲往返已测。早期H1整句后才观察的限制仍保留。 | [17](../.artifacts/full-article-test-20261106/checkpoints/17-six-headings.md)、[lifecycle](../.artifacts/full-article-test-20261106/checkpoints/lifecycle-after-reopen.md) |
| N04 | 段落与换行 | 通过 | 普通段落、Enter、Shift+Enter、两端合并与撤销已观察；反斜杠硬换行逐键保存后，语义视图sun/rain确实分两行。 | [68](../.artifacts/full-article-test-20261106/checkpoints/68-basic-editing.md)、[70](../.artifacts/full-article-test-20261106/checkpoints/70-hardbreak-indented-code.md) |
| N05 | 粗体 | 部分 | 逐键闭合与正向选区组合开关已验证；格式内部和外部续写子项未穷尽。 | [02](../.artifacts/full-article-test-20261106/checkpoints/02-inline.md)、[66](../.artifacts/full-article-test-20261106/checkpoints/66-format-forward.md) |
| N06 | 斜体 | 部分 | 星号、下划线形式均写入；反向组合开关后追加行内代码，保存无交叉标签。 | [02](../.artifacts/full-article-test-20261106/checkpoints/02-inline.md)、[67](../.artifacts/full-article-test-20261106/checkpoints/67-format-reverse.md) |
| N07 | 删除线 | 通过 | 逐键波浪号闭合并显示；源码删一个闭合字符后失去删除线，Undo 后稳定画面恢复。 | [02](../.artifacts/full-article-test-20261106/checkpoints/02-inline.md) |
| N08 | 下划线 | 失败 | 独立 u 标签正常；实体前缀触发保存错序 BUG 02。 | [23c](../.artifacts/full-article-test-20261106/checkpoints/23c-underline-closed.md)、[24](../.artifacts/full-article-test-20261106/checkpoints/24-entity-underline.md) |
| N09 | 高亮 | 部分 | mark 标签逐键形成；后续正文编辑未单独验证。 | [25](../.artifacts/full-article-test-20261106/checkpoints/25-highlight.md) |
| N10 | 上标 | 部分 | sup 标签形成并显示；未覆盖全部别名及历史操作。 | [26](../.artifacts/full-article-test-20261106/checkpoints/26-scripts.md) |
| N11 | 下标 | 部分 | sub 标签形成并显示；未覆盖全部别名及历史操作。 | [26](../.artifacts/full-article-test-20261106/checkpoints/26-scripts.md) |
| N12 | 行内代码 | 部分 | 反引号逐键闭合、格式组合后追加成功；删除闭合符后显示未闭合字面，Undo恢复代码样式。内部更多字面符号未单独验证。 | [21](../.artifacts/full-article-test-20261106/checkpoints/21-inline-link.md)、[67](../.artifacts/full-article-test-20261106/checkpoints/67-format-reverse.md)、[71](../.artifacts/full-article-test-20261106/checkpoints/71-list-history.md) |
| N13 | 链接 | 失败 | 四类链接均写入；前两条文末续段路径损坏链接BUG03，78a2→78b稳定等待对照未复现、78c撤销完全恢复。触发未收敛，后半段中间态还有批次观察限制。 | [62](../.artifacts/full-article-test-20261106/checkpoints/62-link-end-boundary.md)、[63b](../.artifacts/full-article-test-20261106/checkpoints/63b-link-end-repro.md)、[64](../.artifacts/full-article-test-20261106/checkpoints/64-link-variants.md)、[78a2](../.artifacts/full-article-test-20261106/checkpoints/78a2-stable-link-exact-eof.md)、[78b](../.artifacts/full-article-test-20261106/checkpoints/78b-stable-link-after.md)、[78c](../.artifacts/full-article-test-20261106/checkpoints/78c-stable-link-restored.md) |
| N14 | 转义与实体 | 失败 | 转义星号及实体可显示；实体与u标签组合触发BUG02。反斜杠删除/Undo后保存串恢复，删除态视觉受光标遮挡；方括号转义未单独补齐。 | [22](../.artifacts/full-article-test-20261106/checkpoints/22-escape-underline.md)、[24](../.artifacts/full-article-test-20261106/checkpoints/24-entity-underline.md)、[69](../.artifacts/full-article-test-20261106/checkpoints/69-symbols.md)、[71](../.artifacts/full-article-test-20261106/checkpoints/71-list-history.md) |
| N15 | Emoji | 部分 | 逐键数字实体渲染为表情并保留；违规系统选择器路径排除，77中原Unicode表情已清除。实体不等于原始Unicode/IME或应用Emoji菜单入口。 | [54](../.artifacts/full-article-test-20261106/checkpoints/54-entity-and-table.md)、[77](../.artifacts/full-article-test-20261106/checkpoints/77-picker-emoji-removed.md) |
| N16 | 无序列表 | 部分 | 逐键形成、续项、空项退出及历史已测；跨嵌套选区 Tab 未观察到缩进，Shift+Tab 提升后 Undo 恢复。复杂子树未穷尽。 | [03](../.artifacts/full-article-test-20261106/checkpoints/03-lists.md)、[71](../.artifacts/full-article-test-20261106/checkpoints/71-list-history.md) |
| N17 | 有序列表 | 部分 | 数字/点/空格及嵌套逐键形成；类型切换与两次Undo恢复已测，相邻列表发生并组；独立编号续项未补齐。 | [03](../.artifacts/full-article-test-20261106/checkpoints/03-lists.md)、[71](../.artifacts/full-article-test-20261106/checkpoints/71-list-history.md) |
| N18 | 任务列表 | 通过 | 逐符号创建、UI 勾选后 Enter 续项；保存原项 x、新项空格。 | [04](../.artifacts/full-article-test-20261106/checkpoints/04-tasks.md) |
| N19 | 引用 | 部分 | 普通、嵌套、软换行与退出已覆盖；空引用删除与恢复未测。 | [05](../.artifacts/full-article-test-20261106/checkpoints/05-quotes.md) |
| N20 | 提示块 | 失败 | 五类均有逐键样本；空 NOTE 直接续写落到块前 BUG 01。 | [20a](../.artifacts/full-article-test-20261106/checkpoints/20a-callout-prefix.md)、[20b](../.artifacts/full-article-test-20261106/checkpoints/20b-callout-body.md)、[39](../.artifacts/full-article-test-20261106/checkpoints/39-callout-boundary.md) |
| N21 | 代码块 | 部分 | 反引号首行x、Ctrl+Enter块外y及波浪号补测正确；四空格x已保存并显示代码块，格内符号/缩进/Undo已测。闭合模板全部前后边界及空块删除历史未覆盖。 | [07](../.artifacts/full-article-test-20261106/checkpoints/07-code-exit.md)、[61](../.artifacts/full-article-test-20261106/checkpoints/61-tilde-and-anchor.md)、[70](../.artifacts/full-article-test-20261106/checkpoints/70-hardbreak-indented-code.md)、[72](../.artifacts/full-article-test-20261106/checkpoints/72-code-editing.md) |
| N22 | Markdown 表格 | 部分 | 两种手写管道表及UI插表已测；普通表Tab/ShiftTab、增删行列/Undo、表后续写通过。首次瞬态拆行仍留观察，对齐属性未单独操作。 | [12](../.artifacts/full-article-test-20261106/checkpoints/12-table.md)、[13b](../.artifacts/full-article-test-20261106/checkpoints/13b-table-resave.md)、[73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md)、[76](../.artifacts/full-article-test-20261106/checkpoints/76-table-outside.md) |
| N23 | HTML 合并表格 | 部分 | 按当前专用结构逐键建立，跨行 A、跨列 B 与 C/D 显示，格内改为 Ax/By；历史观察不充分。 | [51](../.artifacts/full-article-test-20261106/checkpoints/51-html-table-valid.md)、[52](../.artifacts/full-article-test-20261106/checkpoints/52-merged-cell-edit.md)、[56](../.artifacts/full-article-test-20261106/checkpoints/56-table-history-observation.md) |
| N24 | 本地图片 | 通过 | 逐键路径加载及块后续写已观察；右键明确删除整图，Undo恢复；另观察块边界合并/Undo，未把该路径误称删除图片。 | [08](../.artifacts/full-article-test-20261106/checkpoints/08-image.md)、[73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md) |
| N25 | 行内公式 | 通过 | 逐个美元符号、x+1、闭合并显示；双击重入改为x+2，点击外部提交并观察预览，Undo恢复x+1。另完成整公式删除/Undo。 | [09](../.artifacts/full-article-test-20261106/checkpoints/09-math.md)、[73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md) |
| N26 | 块公式 | 通过 | 空块到x+1逐键形成；重入改为x+2、外部点击提交并观察预览，Undo恢复x+1。首次Esc取消路径不混算。 | [09](../.artifacts/full-article-test-20261106/checkpoints/09-math.md)、[73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md) |
| N27 | Mermaid | 部分 | 七种图均为逐字符键入且逐图渲染；部分符号截图是批次后查看，重入编辑和历史未覆盖全部类型。 | [38](../.artifacts/full-article-test-20261106/checkpoints/38-flowchart.md)、[45](../.artifacts/full-article-test-20261106/checkpoints/45-seven-diagrams.md) |
| N28 | Details | 通过 | UI插入后title、Enter、直接in保存正确；另有全手写样本展开正确。Ctrl+Enter后out在块外，保存74，Undo后out及外部新段消失、title/in保留。 | [10](../.artifacts/full-article-test-20261106/checkpoints/10-details.md)、[33](../.artifacts/full-article-test-20261106/checkpoints/33-details-handwritten.md)、[74](../.artifacts/full-article-test-20261106/checkpoints/74-details-outside.md) |
| N29 | 安全 HTML | 部分 | span 样式、font、div/p 及实体有样本；末尾续段落点异常观察，部分中间态为批次后查看。 | [14](../.artifacts/full-article-test-20261106/checkpoints/14-html-span.md)、[58](../.artifacts/full-article-test-20261106/checkpoints/58-font-boundary.md)、[59](../.artifacts/full-article-test-20261106/checkpoints/59-html-container.md) |
| N30 | 行内注释 | 部分 | seed 与 grow 间的 note 注释逐键形成；重入编辑未测。 | [27](../.artifacts/full-article-test-20261106/checkpoints/27-inline-comment.md) |
| N31 | 块注释 | 部分 | 未闭合注释内换行观察异常；源码逐键形成三行注释显示正确。 | [28](../.artifacts/full-article-test-20261106/checkpoints/28-block-comment.md)、[29](../.artifacts/full-article-test-20261106/checkpoints/29-multiline-comment.md) |
| N32 | 脚注 | 部分 | 引用与定义逐键写入，初次正文落在定义外，UI 源码补齐后显示；跳转与往返未测。 | [30](../.artifacts/full-article-test-20261106/checkpoints/30-footnote.md)、[32](../.artifacts/full-article-test-20261106/checkpoints/32-footnote-complete.md) |
| N33 | 分割线 | 部分 | 三个短横线逐键形成分割线，并在其后继续写 span；块前编辑未单独验证。 | [14](../.artifacts/full-article-test-20261106/checkpoints/14-html-span.md) |

## 基础写作结果

共 45 项：通过 24 项、失败 1 项、部分 20 项。同一行包含多个子场景时，只完成其中一部分就记“部分”。

| ID | 状态 | 本轮结果与未覆盖范围 |
| --- | --- | --- |
| W01 | 通过 | 从已知编辑焦点Ctrl+N，观察新标签后未点击正文直接逐键abc成功；初次主页焦点不明的尝试保留观察，不混算。 |
| W02 | 通过 | one two 的字符/词、Home/End、正反选区均观察。 |
| W03 | 通过 | 选区替换 X、Backspace/Delete，稳定画面确认 Ctrl+Z/Ctrl+Y。 |
| W04 | 通过 | Enter 分段、Shift+Enter 存为 br；段首退格及段尾 Delete 合并均撤销恢复。 |
| W05 | 部分 | 源码全文替换X→Undo→Redo→Undo逐步保存核对：X均为1字节，两次恢复均与1968字节原文完全一致，scratch始终abc。X后光标可见在末尾；恢复后全文选区范围未可靠核实。 |
| W06 | 通过 | 完整符号串逐符号停顿，源码核对无丢字；末单反引号作为未闭合字面状态保留。 |
| W07 | 通过 | 真实拼音 huayuan 候选提交花园；nihao 组字后 Esc 取消，无残留。 |
| W08 | 通过 | 独立剪贴板：自写Review复制后普通/纯文本粘贴均得到ReviewReview，各Undo恢复；剪切后消失、同位置粘回Review。仅普通文字，未覆盖富文本差异，不计逐键输入证据。 |
| H01 | 部分 | H1及后续正文已逐键写入，另有H2–H6；最初标题与第一句正文在整句后观察，不能据此宣布全部中间态通过。 |
| H02 | 通过 | Morning开头Enter生成前置空H2、原标题保留，Undo恢复；Ctrl+3改级别后Undo恢复H2。 |
| H03 | 通过 | 大纲点击H1、Morning/H2，两栏均到对应标题；展开Morning后点Soil/H3，左侧显示Soil、右侧光标在### Soil。 |
| F01 | 部分 | 粗体形成和正向 Ctrl+B/I/I/B 组合通过；格式内外全部子项未测。 |
| F02 | 部分 | 两种斜体写入，反向组合后 body 加行内代码保存正确；格式内部子项未穷尽。 |
| F03 | 通过 | 删除线与行内代码逐键形成；分别删除一个闭合字符后变未闭合状态，Ctrl+Z 后稳定画面恢复样式。 |
| F04 | 部分 | 行内链接写入，标签内部改字未测；文末 Ctrl+End/Enter 失败另记 BUG 03。 |
| F05 | 部分 | 转义写入，signs 中反斜杠删除与Undo已做，保存串与69完全一致；细小符号被光标遮挡，删除态视觉证据有限，方括号转义子项未补齐。 |
| L01 | 部分 | 普通段后前缀形成与嵌套；两个独立初态未完整记录。 |
| L02 | 部分 | 1、点、空格与嵌套输入成功；两个独立初态未完整记录。 |
| L03 | 部分 | 空格入口已测；独立 Tab 前缀入口未测。 |
| L04 | 部分 | water 末尾 Enter 续空项、再次 Enter 退出，Ctrl+Z 恢复空项、Ctrl+Y 再退出；有序编号路径未单独补测。 |
| L05 | 部分 | 末项与空子项缩进有观察；首项/末项/空项三初态未完整记录。 |
| L06 | 部分 | 子项 Tab/Shift+Tab 已做；带孙项子树与顶层退出未测。 |
| L07 | 部分 | water 内切有序后 plant/water 与相邻 dig 并成1/2/3，再切无序；两次Undo恢复原分组。未做完整三同级项含勾选子项的独立初态。 |
| L08 | 通过 | 任务创建、勾选、Enter 新未勾选项，保存字节一致。 |
| L09 | 部分 | 正向选区跨 plant/子项seed/water，Tab 稳定帧未见缩进；Shift+Tab 后顶层变普通段、seed提升，Undo恢复。反向选区和预算边界未测。 |
| L10 | 部分 | water项首Backspace退出为普通段，Undo恢复无序项，plant/seed保留；粗体或链接正文内Enter子场景未测。 |
| Q01 | 通过 | 普通/嵌套引用，Enter 与 Shift+Enter 的保存结构已核对。 |
| Q02 | 部分 | 引用末尾退出已测；空块 Backspace/Delete 与撤销未测。 |
| Q03 | 失败 | 手写空 NOTE 后 Enter 输入落块前，BUG 01。 |
| C01 | 通过 | txt 后只按一次 Enter，首个代码行就是 x。 |
| C02 | 通过 | 语义代码节点内输入字面*、Enter，Tab/Shift+Tab后Undo恢复缩进；72保存x*及两个空格的下一行，围栏与块外y保留。 |
| C03 | 部分 | 未闭合手写围栏 Ctrl+Enter 自动闭合且 y 在外；闭合模板前后插段和空块历史未补。 |
| T01 | 通过 | UI网格2×2插入后逐格p/q/3/4，75保存位置正确，旧a/b表保留；当前实现另加表头，因此总计三行，尺寸含义见观察。 |
| T02 | 部分 | 普通表Tab a→b、Shift+Tab b→a稳定；合并表格编辑已做，但合并格历史截图不充分，导航边界未穷尽。 |
| T03 | 通过 | 普通表UI增行/增列各Undo、删数据行/首列各Undo，稳定画面及73保存恢复a/b、1/2；76保存t在新表之后、旧表之前的普通段。 |
| O01 | 部分 | 本地图片逐键路径、加载、块后继续输入及右键整图删除/Undo已测；对象前新插文字未独立复测。 |
| O02 | 通过 | 行内与块公式分别双击重入，x+1改x+2，外部点击提交、预览确认后Ctrl+Z恢复。未将首次Esc取消后的Undo计入有效路径。 |
| O03 | 通过 | title Enter直接in无鼠标/方向干预；74独立保存证明out在details外。随后Undo稳定复看out及外部新段消失，title/in保留。 |
| R01 | 通过 | 查询body加尾空格得到2处，Enter从1/2到2/2，Shift+Enter回1/2；随后正文焦点Undo及Review选择/复制可用。 |
| R02 | 通过 | body加尾空格共2处；单次仅第一行变stem，正文焦点Undo恢复；全部替换两行变stem、查询无结果，再正文Undo恢复两body。 |
| R03 | 通过 | 查询zzzq显示无结果，清空后稳定；选中Review按Ctrl+F自动带入查询，显示0/1，正文未被查询输入修改。 |
| S01 | 部分 | 语义/源码/双栏编辑与Undo均有路径；一次右侧Review编辑/Undo后左侧从末尾跳H1。受控对照左右先定位Review，再x/Undo，内容和位置均保持，未复现原跳动；观察保留。 |
| S02 | 通过 | scratch.md首字a→A出现修改标记，Undo回abc并消除标记；主文Review首字R→r再Undo恢复。文件核对主文仍与78a2整文件一致、辅助文档恰为abc。 |
| S03 | 通过 | 保存关闭主标签、在当前窗口重开原文件再保存，前后1968字节及SHA256完全一致；标题/目录/图/公式/表格等代表结构复看，未宣称所有节点重新逐一视觉验收。 |
| S04 | 通过 | 两测试文档保存后UI Ctrl+Q；主代理20:39核对PID13500已结束，stdout/stderr均0字节，固定二进制和448个源码输入哈希不变。 |

## 缺陷与体验观察

### BUG 01 手写空 NOTE 后输入落到前一段

严重程度：P2。有效复现 2/2；另一次因为前缀身份与画面不一致，不计入有效分母。涉及 N20、Q03。

1. 在语义模式的普通段落 `case` 后取得空白段。
2. 分别按 `>`、空格、`[`、`!`、`N`、`O`、`T`、`E`、`]`。闭合后出现 NOTE 卡片，光标却疑似回到 `case` 末尾。
3. 不点击鼠标或按方向键，直接 Enter，再逐键 `rain`，Ctrl+S。
4. 预期 `rain` 成为 NOTE 正文；实际保存为 `case`、普通段落 `rain`、空的 `> [!NOTE]`，输入落在提示块之前。

第一次样本见 [18-callout-entry.md](../.artifacts/full-article-test-20261106/checkpoints/18-callout-entry.md)。受控复现分成两个磁盘检查点：[20a-callout-prefix.md](../.artifacts/full-article-test-20261106/checkpoints/20a-callout-prefix.md) 确认第二个前缀已保存，[20b-callout-body.md](../.artifacts/full-article-test-20261106/checkpoints/20b-callout-body.md) 的差异只是在它前面增加 `rain` 段。

影响是手写空提示块后无法沿当前落点直接写正文；本轮没有发现该路径丢失 `rain` 字符。根因尚未确认，未修改产品代码。需在后续修复中复测空提示块形成后的光标锚点，以及 Enter 到正文的路径。

### BUG 02 实体前缀后的下划线标签保存错序

严重程度：P2。涉及 N08、N14。原混合段和精简实体前缀样本各出现一次；无实体前缀的独立段对照正常。

在语义模式空段逐字符输入 `&amp; <u>leaf</u>`，所有输入通过真实按键完成；保存后实际得到 `&amp; leaf</u><u>`。开标签移到了正文和闭标签之后，与按键顺序不符，保存源码的下划线语义损坏。

- 原混合行证据：[22-escape-underline.md](../.artifacts/full-article-test-20261106/checkpoints/22-escape-underline.md)，此前还有行内代码、链接和转义符。
- 无实体对照分三阶段读取磁盘：[开标签](../.artifacts/full-article-test-20261106/checkpoints/23a-underline-open.md)、[正文](../.artifacts/full-article-test-20261106/checkpoints/23b-underline-body.md)、[闭标签](../.artifacts/full-article-test-20261106/checkpoints/23c-underline-closed.md)，最终为正确的 `<u>leaf</u>`。
- 精简复现：[24-entity-underline.md](../.artifacts/full-article-test-20261106/checkpoints/24-entity-underline.md)，仅保留 `&amp; ` 前缀即可出现同样错序。

最初混合段的闭标签字母与最后尖括号之间没有单独截图，但仍是两个独立按键；精简复现补齐了逐符号观察。实体前缀是本轮定位到的触发条件，尚未证明底层根因。原混合段已通过 UI 源码编辑恢复正确标签，精简复现的原始样本另行保留；产品代码没有修复。

### BUG 03 语义文末链接导航后续写曾破坏链接，触发条件未完全收敛

严重程度：P2。涉及 N13 的文末边界。前两个有效路径均出现异常；第二次将后续内容缩小为普通字母，排除了新输入 HTML 的干扰。后续逐步等待稳定画面的对照没有复现，不能把它描述成必现问题。

1. 文件末尾为完整的 `[top](#morning)`，没有尾部空段。
2. 切到语义模式，Ctrl+End，Enter，再逐键输入 `z`，不以鼠标或方向键补救。
3. 预期在完整链接之后建立普通段落并写入 `z`。
4. Ctrl+End 后光标看起来位于可见 `top` 末尾；保存却变成 `[top`、空行、`z](#morning)`，新增内容出现在原链接结束标记之前。没有记录内部光标字节偏移，不能仅凭画面确定是哪一步改变了插入落点。

首次输入自动链接的异常见 [62-link-end-boundary.md](../.artifacts/full-article-test-20261106/checkpoints/62-link-end-boundary.md)。受控复现的[正确初态](../.artifacts/full-article-test-20261106/checkpoints/63a-link-restored.md)与[单字复现](../.artifacts/full-article-test-20261106/checkpoints/63b-link-end-repro.md)已分别保存，其差异仅涉及末尾链接。

补充对照先确认 [78a2](../.artifacts/full-article-test-20261106/checkpoints/78a2-stable-link-exact-eof.md) 恰以 `[t](#morning)` 结束，无尾空格或换行；最初准备时留下的四个尾空格已通过 UI 去掉，该未满足条件的初态不计测试结果。切语义后依次 Ctrl+End、观察稳定画面、Enter、再观察、键 z，[78b](../.artifacts/full-article-test-20261106/checkpoints/78b-stable-link-after.md) 保存为完整链接、空行及块外 z，没有破坏链接。三条有效路径中观察到两次失败、一次未复现；除等待节奏外，标签及文档状态也有差异，不能据此断言“等待即可解决”或已经确定为时序问题。下一轮应从 63a 的原样本分别对照连续输入与稳定等待。

代码排查入口是 `nomo_mark/keyboard.rs` 的 `document_edge` 与 `resolve_navigation_caret`：前者请求文档边界，后者把语义光标校正到最近的可见落点。本轮没有追踪内部字节偏移，不能据此确定根因；也没有证明它与 HTML 结束标签边界或空 NOTE 的问题共用根因。

### 体验观察与无效尝试

以下记录不与三项已确认缺陷合并计数。单次现象保留复测入口；输入错误、源码协议不符和截图取到旧帧分别说明。

| ID | 起点、操作与实际观察 | 证据与判定边界 |
| --- | --- | --- |
| OBS 01 | 首个无空格管道表逐键输入完成，保存源码为连续的表头、分隔行和数据行；在块后操作与撤销后，语义画面曾将数据行显示为字面文本。后续切换模式后恢复。 | [12-table.md](../.artifacts/full-article-test-20261106/checkpoints/12-table.md)、[13-table-transition.md](../.artifacts/full-article-test-20261106/checkpoints/13-table-transition.md)、[13b-table-resave.md](../.artifacts/full-article-test-20261106/checkpoints/13b-table-resave.md) 字节相同。后续回访时 Tab 导航及增删行列/Undo 正常，[73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md) 原表源码完整；原瞬态仍无稳定最小复现。 |
| OBS 02 | 手写 `<!-- toc -->` 和 `<!-- /toc -->` 后已有 H1–H6；移开光标并多次保存，目录仍显示“文档暂无标题”，保存内容仍是空标记。后续交互阶段才生成六条目录。 | [17](../.artifacts/full-article-test-20261106/checkpoints/17-six-headings.md) 至 [75](../.artifacts/full-article-test-20261106/checkpoints/75-ui-table.md) 仍为空；[76](../.artifacts/full-article-test-20261106/checkpoints/76-table-outside.md) 已有完整 H1–H6 链接，75→76 的差异仅为表后 t 及这六条派生内容。之间做过标题修改/Undo、大纲与模式操作，具体触发未隔离。早先的 `[TOC]` 不属当前语法，不作为失败证据。 |
| OBS 03 | 正确 `<span …>leaf</span>` 后 Enter 再输入 `<`，落点疑似仍在结束标签内；另一次正确 `<font face="serif">leaf</font>` 后 Ctrl+End、Enter、开始 div，保存把新段插到 `</font>` 前。 | span 只有单次画面观察；font 有 [58-font-boundary.md](../.artifacts/full-article-test-20261106/checkpoints/58-font-boundary.md)。通过 UI 源码修整后 [59-html-container.md](../.artifacts/full-article-test-20261106/checkpoints/59-html-container.md) 为独立 font/div。不能据此认定与 BUG 03 同一根因。 |
| OBS 04 | 逐键脚注定义 `[^1]:`、空格、`rain`，正文最终在定义外。 | [30-footnote.md](../.artifacts/full-article-test-20261106/checkpoints/30-footnote.md)；一次观察。通过 UI 源码补齐后 [32-footnote-complete.md](../.artifacts/full-article-test-20261106/checkpoints/32-footnote-complete.md) 保留正确定义。 |
| OBS 05 | 语义模式输入未闭合 `<!--`，Enter 后继续 `soil` 并闭合，结果是单行 `<!--soil-->`，换行留在外面。 | [28-block-comment.md](../.artifacts/full-article-test-20261106/checkpoints/28-block-comment.md)；一次观察。源码模式逐键建立的三行注释见 [29-multiline-comment.md](../.artifacts/full-article-test-20261106/checkpoints/29-multiline-comment.md)。 |
| OBS 06 | 初始主页 Ctrl+N 未观察到明确结果；点击新建后首字 `a` 未进入正文，点编辑区后才能输入。 | 当时焦点和输入法状态不够确定，不计有效缺陷复现。后续从已知编辑焦点 Ctrl+N，不点击新文档正文便能直接输入 abc，保存为 scratch.md。该路径通过，不据此抹去初次主页尝试的不确定性。 |
| OBS 07 | 合并表格格内 A/B 编辑成功；随后删除、撤销、重做的即时截图部分与后续稳定画面不一致。 | [52-merged-cell-edit.md](../.artifacts/full-article-test-20261106/checkpoints/52-merged-cell-edit.md) 有 Ax/By、C/D 与正确 spans；[56-table-history-observation.md](../.artifacts/full-article-test-20261106/checkpoints/56-table-history-observation.md) 已经 UI 恢复 A/B。原历史尝试不能记通过，也不能只凭旧帧判产品失败。 |
| OBS 08 | 在 water 内切有序后，plant/water 与相邻 dig 并为 1/2/3；切无序后共同变无序。两次 Undo 恢复原分组。跨 plant/seed/water 选区 Tab 未见缩进，Shift+Tab 提升后 Undo 恢复。 | [71-list-history.md](../.artifacts/full-article-test-20261106/checkpoints/71-list-history.md) 的内容、子层级和任务状态保留。相邻同类型 Markdown 列表可能并组，且选区包括首项；尚不能把这两项观察判成缺陷。 |
| OBS 09 | UI 网格选“2×2”后显示两列表头和两行数据，逐格写 p/q、3/4 后还剩一行空数据；Ctrl+Enter 插入数据行，Undo 后仍有原来的空行。 | [75-ui-table.md](../.artifacts/full-article-test-20261106/checkpoints/75-ui-table.md) 与当前 `table_insert.rs` 一致：rows 是数据行数，表头额外添加。`view.rs::insert_paragraph` 在语义单元格中明确走插行分支。这两项不是本轮确认的缺陷；尺寸文字未说明是否包含表头，存在理解歧义。 |
| OBS 10 | 进入双栏时左侧在文末 font/div、top 与表附近，右侧在 Review；右侧末尾键 x、Undo 后，左侧跳至 H1，右侧仍在 Review。 | 只有一次观察。随后受控对照先让两栏均显示 Review：右侧键 x 时左侧同步 Reviewx、位置保持，Undo 后两栏回 Review、位置仍保持。该对照未复现，不能将原观察关闭或认定为稳定缺陷；S01 保留为部分覆盖。 |

以下不计产品缺陷：普通 HTML table 缺少 `data-nomo-table="semantic"`、`tbody` 或单元格 `p` 时不走原生合并表格入口；测试者曾输入 `<td<p>>`，之后修正。ER 图首次把关系符的 `o` 输入为 `O`，显示解析错误后改正，正确版本渲染成功。首次波浪号围栏没有按 Enter 就输入 `xx`，保存为语言标记 `~~~xx`，后续按正确顺序得到正文 `x`。NOTE 正文后缺少空行造成的 Markdown 惰性续行也已通过 UI 修整，不能归咎于应用。

图片单击后 Backspace 的一次尝试只合并了块边界，未删除图片；之后使用右键删除菜单才取得整图删除/Undo 证据。公式单击后的 Backspace 曾删去整个对象，Undo 恢复；内部修改使用双击入口。块公式首次按 Esc 取消草稿后又误按 Undo，影响了前一次代码缩进；[72](../.artifacts/full-article-test-20261106/checkpoints/72-code-editing.md) 与 [73](../.artifacts/full-article-test-20261106/checkpoints/73-objects-table-history.md) 仅差代码空行中的两个空格。后续重新执行了明确提交 x+2 再 Undo 到 x+1 的有效路径，错误尝试不计为公式历史缺陷或通过证据。

新标签连续输入 abc 后追加 x，一次 Undo 撤销整组 abcx 回到空白；没有设定连续输入的历史分组时限预期，不判为缺陷，也不误称仅撤回 x。随后用已保存 abc 的首字 a→A 替换/Undo 验证该标签历史，再对主文 Review 的首字做替换/Undo；两份文件均恢复原字节。

静态截图没有覆盖连续帧，不能证明完全无闪烁。部分动作后首次截图仍是旧画面，补取稳定画面再作判定；工具调用时间也不当作应用卡顿时长。

## 保存与收尾

全文历史与重开均使用两阶段取证：UI 保存后暂停，主代理先复制文件和计算哈希，再允许下一步操作。[lifecycle.json](../.artifacts/full-article-test-20261106/evidence/lifecycle.json) 记录七个阶段及辅助文档哈希。

| 阶段 | 主文章结果 | 辅助文档 |
| --- | --- | --- |
| 全文替换前 | 1,968 字节，完整文章快照 | `abc`，3 字节 |
| Ctrl+A 后输入 X 并保存 | 恰为 `X`，1 字节 | 未变 |
| 第一次 Undo 后保存 | 与替换前完全相同 | 未变 |
| Ctrl+Y 后保存 | 再次恰为 `X`，与第一次 X 哈希相同 | 未变 |
| 最后 Undo 后保存 | 再次与替换前完全相同 | 未变 |
| 保存关闭主标签 | 先保留关闭前快照 | 保留辅助标签 |
| 当前窗口重开原文件并保存 | 与关闭前完全相同 | 未变 |

恢复后的文末链接可见高亮，但没有可靠确认整个文档的选区范围；因此 W05 的字节与历史路径通过，整项仍记部分。重开后实际复看了 H1–H6、完整目录、图片、行内/块公式、三张普通表、合并表格 A/B/C/D、花园、实体表情、流程图、时序图和末尾链接；没有把这次代表性复看写成所有节点的再次完整视觉验收。

最终主文章为 **1,968 字节、1,964 个 Unicode 字符、266 行、UTF-8 无 BOM**，SHA-256：

`C85344C4A40C926B117811DA5C6D4ACB678DFFF102FAC15E91CF1C0D2D6AFE7E`

[源码清单](../.artifacts/full-article-test-20261106/evidence/source-audit.json) 确认 H1–H6、七类闭合 Mermaid 围栏和合并表格的 `rowspan=2`、`colspan=2`；原选择器 Unicode 表情数量为 0，数字实体和真实 IME 提交的花园各保留 1 处。两处 `body ` 已恢复、`stem ` 为 0、没有 `ReviewReview` 残留。这些只是保存内容核对，不替代前面的 UI 结论。

本轮保留 **85 份文章检查点**。从 N18 开始的 [ui-actions.jsonl](../.artifacts/full-article-test-20261106/evidence/ui-actions.jsonl) 有 **2,057 条按键记录**，不包括全部早期按键，也不等于整轮操作总数；部分 phase 标签滞后于实际操作，应结合时间、桌面工具历史与保存样本追溯。

两份测试文档保存后执行 UI Ctrl+Q。20:39 的[最终核对](../.artifacts/full-article-test-20261106/evidence/final-verification.json)确认 PID 13500 已结束，stdout/stderr 均为 0 字节，固定二进制与图片哈希未变，448 个构建输入文件没有哈希变化。Git 状态相对开始前仅新增本测试报告；测试脚本、二进制、数据目录及证据位于忽略的 `.artifacts` 中，未提交。

辅助快照脚本曾因 PowerShell 单元素集合展开而追加记录失败；修正后复用了已保存且哈希一致的 X 检查点。该辅助脚本未写主文章，不计为产品失败。

## 性能观察与验证边界

本轮未改产品代码，未引入产品常驻实例、缓存或依赖；测试仅启用已有的有界帧采样。最后落盘时间为 20:37:09，数据见 [frame-profile.json](../.artifacts/full-article-test-20261106/evidence/frame-profile.json)。

| 指标 | 样本数 | 中位数 | p95 | 最大值 |
| --- | ---: | ---: | ---: | ---: |
| 工作区 CPU 渲染至绘制结束 | 11,040 | 1.334 ms | 2.414 ms | 24.653 ms |
| 按键入口至随后 CPU 绘制结束 | 1,561 | 2.469 ms | 3.321 ms | 25.673 ms |

采样每类最多保留 16,384 条，每 240 帧写一次，退出时不额外刷新；因此最后尾段可能未包含。连续按键可能合并为一个待绘制样本，这些数字不是每个物理按键的测量，也不是显示器上屏、IME 端到端或用户体感延迟。它们不能证明没有短暂闪烁、卡顿或泄漏。

未执行三轮性能对照、内存/GPU 测量、多 DPI、多窗口、跨平台、大文档压力测试，也未覆盖每种节点内的 IME 组字、所有嵌套/样式组合或全部应用功能。富文本剪贴板、表格所有导航边界及清单中标明的其它子场景仍未测。下一轮修复复测应使用本轮固定样本，优先复查空 NOTE 续写、实体前缀的 u 标签及 63a 的原始文末链接条件；目录延后更新和双栏阅读位置跳动保留为待复现观察。

## 2026-11-07 两项定向修复与复验

本节记录后续修复；上文的统计和“未修复”描述保留为 11 月 6 日测试时的历史状态。本次只针对 BUG 01、BUG 02，默认 Workspace 入口使用最终冻结的 Release 构建完成原场景复验。

| 问题 | 修复及最终版结果 | 保存证据 |
| --- | --- | --- |
| BUG 01 空 NOTE 续写落到前段 | 空提示块补回正文落点；手写标记后的第一次 Enter 进入正文。逐键 `]` 后出现“提醒”卡，光标留在卡内；Enter 后输入 `rain` 保存在 NOTE 内。 | [NOTE 原场景](../.artifacts/fix-note-underline-20261107/evidence/final-note-after-rain.md) |
| BUG 02 实体前缀后下划线标签错序 | 光标旁尚未配对的行内格式标签保持展开及源码映射。逐键输入后保存为 `&amp; <u>leaf</u>`，正文显示下划线，闭标签位置正确。 | [两个最小场景](../.artifacts/fix-note-underline-20261107/evidence/final-both-minimal.md) |

BUG 01 涉及两处：解析器对只有提示类型标记的块不产生正文段落，显示投影缺少落点；Enter 规则又将这种首行当成退出空提示块。修复位于 `nomo_mark/parse.rs` 和 `editing_rules.rs`，首个正文行复用 `commands.rs` 的引用前缀续写函数。已有空正文行的退出规则保留。

BUG 02 的原因是未配对 `<u>` 没有对应的行内跨度，标签隐藏后没有末尾光标映射，后续输入被放到开标签之前。`markers_for` 现在为光标附近未配对的格式标签补充展开范围；已配对标签继续使用原来的展开及表格原子对象规则。没有改动原文实体编码。

最终版本通过以下既有检查，全部退出码为 0，未新增测试用例：

- `cargo check --locked --bin nomo`。
- `cargo check --locked --bin nomo --features nomo-mark-demo`。
- Release / `nomo-mark-demo` 下串行执行解析 8 项、编辑命令 7 项、行内格式 5 项、引用退出及会话历史 1 项，共 21 项。
- `cargo build --locked --release --bin nomo --target-dir .artifacts/fix-note-underline-20261107/target`。

命令、时间与日志见 [checks.json](../.artifacts/fix-note-underline-20261107/evidence/checks.json)。最终 [nomo.exe](../.artifacts/fix-note-underline-20261107/bin-final/nomo.exe) 为 70,009,344 字节，SHA-256 为 `07C20A5D6BEF7BF46582460E99501744316646D1636916F5A4B0DF71849D5D4C`。所记录的 289 个构建输入在编译前后哈希一致，见 [build.json](../.artifacts/fix-note-underline-20261107/evidence/build.json)。

增量开销限于当前解析窗口的标签检查，以及引用投影临时栈中的一个布尔值和必要空段落事件；没有新增全文镜像、常驻缓存、后台任务或依赖，沿用原有文档事务与撤销历史。未开展三轮性能对照或跨平台验收。

最终版还完成了以下相邻操作与持久化检查：

| 检查 | 结果与证据 |
| --- | --- |
| 提示块退出 | 已有正文的 NOTE 末尾空行按 Enter，光标进入块外普通段落；保存源码保留 `rain`。空提示块整体退出另由既有会话测试覆盖。 |
| 下划线对照及续写 | 无实体前缀的 `<u>leaf</u>` 正常；正文删除末字 `f` 后显示 `lea`，再输入恢复 `leaf`；源码与下划线显示均正确。 |
| 撤销重做 | 段外输入 `X` 后保存；Ctrl+Z 撤回该段，两个格式样本及 NOTE 完整保留；Ctrl+Shift+Z 恢复。重做后的源码与撤销前逐字一致：[前](../.artifacts/fix-note-underline-20261107/evidence/final-history-before-undo.md)、[撤销](../.artifacts/fix-note-underline-20261107/evidence/final-history-undo.md)、[重做](../.artifacts/fix-note-underline-20261107/evidence/final-history-redo.md)。 |
| 保存重开 | 关闭标签并通过文件对话框重新打开，NOTE 内 `rain`、两行下划线及末尾 `X` 完整显示；重开后再次保存的哈希与重做时相同。最终文章：[final-note.md](../.artifacts/fix-note-underline-20261107/documents/final-note.md)。 |

最终有效回合使用英文输入状态和实际按键；语法符号逐个输入、观察后再继续。早期 [note-first.md](../.artifacts/fix-note-underline-20261107/documents/note-first.md) 缺少 `>`，[note-prefix.md](../.artifacts/fix-note-underline-20261107/documents/note-prefix.md) 输入成中文 `》`；两份均保留为无效输入样本，不计入产品通过或失败结论。

实机环境为 Windows、Nomo 0.6.2、浅色主题、截图 1282×821，DPI 未测。截图保存在工具历史，未生成独立图片文件。本次覆盖语义模式；双栏语义侧、真实中文 IME、其他 DPI/主题及普通引用桌面专项未复验。

候选实例 PID 26852、最终实例 PID 23124 均正常退出；没有操作用户原有 Nomo 窗口。最终核对确认二进制哈希及所记录的源码输入未变化，重开后的文章为 60 字节，stdout/stderr 均为空，见 [final-verification.json](../.artifacts/fix-note-underline-20261107/evidence/final-verification.json)。产品代码与文档修改未提交或推送。
