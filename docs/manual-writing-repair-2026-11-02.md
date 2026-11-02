# 2026-11-02 四项基础写作缺陷修复复测

**BUG-01 至 BUG-04 的原始复现路径均通过。** Windows 原生 Release 中逐键创建了 8 份新文档，10 个独立保存检查点逐字节核对符合预期。

原始失败样本、45 项清单的原判定及其它观察见[首轮逐键报告](manual-writing-retest-2026-11-02.md)。本轮只复测 BUG-01 至 BUG-04 及相邻对照。所有正文和手写语法均从新的空白文档逐字符、逐符号输入；检查保存文件不作为输入手段。

| 原问题 | 修复位置及行为 |
|---|---|
| BUG-01：粗斜体取消后交叉标签 | 取消格式时区分可见正文与其它嵌套格式的定界符。仅为选区两侧确有正文的部分保留格式，选中的 `body` 不再因外围星号被重建空 HTML 标签。 |
| BUG-02：手写围栏首行空白 | 开围栏尚无换行时也能识别语言行。第一次 Enter 只写入一个本地换行及必要的容器前缀；输入语言名仍可正常修改。 |
| BUG-03：Details 标题 Enter 未进入正文 | 应用只有光标变化的命令并通知宿主，不创建空的内容事务。打开折叠块，在空正文绘制与键盘一致的落点；后续字符可直接在正文输入。 |
| BUG-04：未闭合围栏无法写块后段落 | 在手写围栏末尾执行块后插段时，同一笔编辑先写入对应种类和长度的闭围栏，再把光标移到块外段落；已有闭围栏不重复写入。 |

构建为 Windows 11 `10.0.26200` 上的 Nomo 0.6.2 默认 Release，分支 `gpui-editor`、HEAD `35b2b047` 加工作区改动。新二进制 SHA-256 `058432C227497DC4441002444EBBB76058E9322095F6B064D6799F822449CBC3`，原失败版为 `17FADE0CA49E5AFE3A3EBBD1EAB3AB7FD535E2DBFFCC4F0136904542A7716DFF`。隔离数据、构建日志、输入哈希及本轮新样本见[任务目录](../.artifacts/writing-basic-repair-20261102-01/)。

默认和 `nomo-mark-demo` 的 `cargo check --locked --bin nomo`、默认 `cargo build --locked --release --bin nomo` 通过。7 组已有 Release 测试通过 47 项（行内命令 5、编辑命令 7、窗口宿主 5、模板投影 5、解析 8、方言 2、列表 15）；未新增测试代码。修复限于现有编辑窗口、选区和局部补丁，没有新增全文副本、依赖、常驻任务或缓存。

## 桌面动作与保存结果

| 项目 | 实际操作和判定依据 |
|---|---|
| BUG-01，F01/F02 | 两个新文档分别用 Home → Shift+End 和 End → Shift+Home 选中逐键写出的 `body`。按 Ctrl+B → Ctrl+I → Ctrl+I → Ctrl+B，按 End、空格，再逐键输入反引号、x、反引号。四个中间态依次为粗体、粗斜体、粗体、普通；End 后未出现残留格式。[正选样本](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-01-forward.md)和[反选样本](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-01-reverse.md)磁盘内容逐字节相同：只有普通 body、空格和行内代码 x，共 8 字节，无 HTML。一次中文候选开启时的尝试已清空，不计入这两次结果。 |
| BUG-02，C01 | 三次独立反引号后出现空代码框；依次 t、x、t 后标签为 Plain Text，Enter、x，x 显示在第一行。此时暂停并冻结[首行检查点](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-02-first-line.md)：打开围栏与 x 之间只有一个 LF，8 字节。 |
| BUG-03，O03 | 菜单插入空 Details，逐键 title，在标题末按 Enter，不再点鼠标或按方向键，直接输入 i、n。光标和 in 均出现在框内第二行；[块内检查点](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-03-inside.md)为 `<summary>title</summary>`、空行、in、`</details>`。保持该位置，Enter 两次再逐键 out，[退出检查点](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-03-outside.md)中 out 位于闭标签之后。 |
| BUG-04，C03 | 从上一个检查点保持代码内光标，Ctrl+Enter → y；画面中 y 在框外。[块后检查点](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-04-after-exit.md)为 15 字节，依次是开围栏、x、独占一行的三个闭合反引号、空行、y。 |

相邻操作对照：

- [第二个 Details 文档](../.artifacts/writing-basic-repair-20261102-01/checkpoints/BUG-03-repeat.md)：新建、标题 Enter 后直接输入 in。Ctrl+Z 撤掉正文输入但保留 title；Ctrl+Y 恢复 in；随后两次 Enter、out。保存内容与第一份退出样本逐字节相同。稳定画面未见红字，不能由此关闭首轮的撤销瞬态观察。
- [四波浪号手写围栏](../.artifacts/writing-basic-repair-20261102-01/checkpoints/control-four-fence.md)：四次独立 Shift+反引号，Enter、x、Ctrl+Enter、y；首行 x 在块内，y 在块外，开闭行分别都是四个波浪号。
- [已闭合菜单模板](../.artifacts/writing-basic-repair-20261102-01/checkpoints/control-menu-closed.md)：Ctrl+Shift+K 插入 TypeScript 代码块，x、Ctrl+Enter、y。仅有模板原先的一道闭围栏；界面和源码均确认 y 为块后普通段落。
- [编辑语言名和第二次换行](../.artifacts/writing-basic-repair-20261102-01/checkpoints/control-language-backspace.md)：三个反引号后逐键 txz，Backspace 删除 z，再输入 t、Enter、a、Enter、b。保存的语言名为 txt，正文只有两行 a、b；手写块保持未闭合，普通代码换行不会提前退出。

## 范围

隔离实例 PID 9476 已通过界面选择“关闭窗口”正常退出，stderr 为 0 字节。[按键日志](../.artifacts/writing-basic-repair-20261102-01/key-actions.jsonl)有 209 条真实按键记录，含快捷键、保存和关闭操作。没有使用整段输入或粘贴代替手写正文。主代理独立核验 10 个检查点、8 份草稿、386 个构建输入和二进制哈希，工作区 `target/release/nomo.exe` 与桌面复测副本一致；详见[最终核验 JSON](../.artifacts/writing-basic-repair-20261102-01/final-verification.json)。

本轮从空白页重跑了上述缺陷路径并检查了保存文件。没有重跑完整 45 项，没有重新判定前轮其它观察（包括撤销瞬态红字和双栏短暂不同步）。本轮不涉及历史异常文件自动修复、长文跨窗口、全部 IME 上下文、不同 DPI/平台或量化性能验收。
