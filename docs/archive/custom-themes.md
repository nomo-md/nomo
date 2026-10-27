# 自定义主题与字体

Nomo 的配色主题是一份 JSON 文件。你可以从内置主题导出模板，改颜色后再导入；也可以只写想改的几个颜色，其余从内置主题继承。字体可以在设置里直接覆盖，不需要改主题。

## 快速开始

1. 打开 **偏好设置 → 外观**，选中一个接近你想要效果的内置主题。
2. 点击 **导出当前主题为模板…**，保存 JSON 文件。文件里列出了全部颜色和样式令牌。
3. 用任意文本编辑器修改：
   - 把 `id` 改成你自己的名字，必须以 `user-` 开头，例如 `user-rose`；
   - 修改 `localizedNames` 中的显示名称；
   - 修改 `variants.light` / `variants.dark` 里的颜色。
4. 回到设置，点击 **导入主题…** 选择该文件。导入成功后会自动选中，主窗口同步生效。

修改后用同一个 `id` 再次导入即可覆盖旧版本。将鼠标悬停在自定义主题卡片上，右上角会出现删除按钮；删除只移除 Nomo 内保存的副本，不影响你的原始文件。

## 最小示例

只写需要覆盖的令牌，其余从 `baseThemeId` 指定的内置主题继承：

```json
{
  "schemaVersion": 1,
  "id": "user-ocean",
  "localizedNames": { "zh-CN": "海洋", "en-US": "Ocean" },
  "baseThemeId": "nomo-github",
  "variants": {
    "light": {
      "tokens": { "accent": "#0077b6", "accentFill": "#0077b6", "link": "#0077b6" }
    },
    "dark": {
      "tokens": { "accent": "#48cae4", "accentFill": "#0096c7", "link": "#48cae4" }
    }
  }
}
```

## 文件结构

| 字段             | 必填 | 说明                                                                                                                                                                      |
| ---------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `schemaVersion`  | 是   | 固定为 `1`。                                                                                                                                                              |
| `id`             | 是   | 以 `user-` 开头，只含小写字母、数字和单个连字符，不超过 64 个字符，例如 `user-solarized-2`。同 `id` 再次导入会覆盖。                                                      |
| `localizedNames` | 是   | 显示名称，键为语言代码，如 `zh-CN`、`zh-TW`、`en-US`、`ja-JP`；至少一个。界面语言没有对应名称时依次回退到 `en-US` 和 `id`。                                               |
| `version`        | 否   | 版本号文本，默认 `1.0.0`。                                                                                                                                                |
| `author`         | 否   | 作者，默认 `User`。                                                                                                                                                       |
| `baseThemeId`    | 否   | 用来补齐缺省令牌的内置主题：`nomo-default`（默认）、`nomo-amber-paper`、`nomo-classic-gray`、`nomo-github`。                                                              |
| `styleProfile`   | 否   | 造型档案：`modern`、`paper`、`classic`，默认与基础主题相同。目前只有 `classic` 有额外效果：行内代码改用代码块的边框、背景和文字色。圆角、间距、阴影等造型由样式令牌决定。 |
| `variants`       | 否   | 包含 `light` 和 `dark` 两个变体，均可省略；省略的变体完全使用基础主题。                                                                                                   |

每个变体可包含：

| 字段          | 说明                                                                                                                                                                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tokens`      | 颜色令牌，见下文。                                                                                                                                                                 |
| `styleTokens` | 圆角、间距、阴影、字体等样式令牌，见下文。                                                                                                                                         |
| `shikiTheme`  | 代码高亮主题，可选 `github-light`、`github-dark`、`gruvbox-light-medium`、`gruvbox-dark-medium`。                                                                                  |
| `mermaid`     | `{ "theme": "default" \| "dark" \| "base" }`。选 `base` 时图表颜色由本主题的令牌自动生成（背景、节点、连线、文字），推荐自定义主题使用；`default` / `dark` 使用 Mermaid 自带配色。 |

预览卡片的颜色由 `background`、`surface`、`accent`、`foreground` 自动生成，不需要单独填写。

## 颜色令牌

颜色值可以是任意 CSS 颜色：`#rrggbb`、`rgb()`、`rgba()`、`hsl()`、`transparent` 等。

### 界面与正文

| 令牌                 | 作用                                                 |
| -------------------- | ---------------------------------------------------- |
| `background`         | 窗口整体背景。                                       |
| `surface`            | 卡片、面板、输入框等表面。                           |
| `rail`               | 侧栏背景。                                           |
| `chrome`             | 工具栏等外框区域。                                   |
| `documentBackground` | 正文编辑区背景。                                     |
| `sidebarActive`      | 侧栏、大纲中当前项的高亮背景。                       |
| `foreground`         | 正文与主要文字。                                     |
| `mutedForeground`    | 次要文字、说明、占位符。                             |
| `border`             | 通用边框和分隔线。                                   |
| `selection`          | 文本选区背景，建议用半透明色。                       |
| `hoverBackground`    | 悬停背景。                                           |
| `heading`            | 标题文字。                                           |
| `link`               | 链接文字。                                           |
| `accent`             | 强调色：焦点、选中标记、进度等。                     |
| `accentStrong`       | 更强的强调色，用于需要更高对比度的强调。             |
| `accentFill`         | 主按钮等实心填充背景。                               |
| `onAccent`           | 位于 `accentFill` 上的文字颜色，需与之保持足够对比。 |
| `success`            | 成功状态。                                           |
| `warning`            | 警告状态（如“未检测到字体”提示）。                   |
| `danger`             | 错误、删除等危险状态。                               |

### 引用块与提示块

`blockquote*` 控制引用块，`callout*` 控制 GitHub 风格提示块（`> [!NOTE]` 等）。名称中带 `Classic` 的背景只在 **文档样式 = 经典** 时使用；现代样式使用不带 `Classic` 的背景（内置主题多为 `transparent`）。

| 令牌                                                                     | 作用                              |
| ------------------------------------------------------------------------ | --------------------------------- |
| `blockquoteBorder` / `blockquoteForeground`                              | 引用块左边线 / 文字。             |
| `blockquoteBackground` / `blockquoteClassicBackground`                   | 引用块背景（现代 / 经典）。       |
| `calloutNoteBorder` / `calloutNoteForeground`                            | NOTE 左侧强调条 / 文字。          |
| `calloutNoteBackground` / `calloutNoteClassicBackground`                 | NOTE 背景（现代 / 经典）。        |
| `calloutTip*`、`calloutImportant*`、`calloutWarning*`、`calloutCaution*` | 其余四类提示块，后缀含义同 NOTE。 |

### 代码与表格

| 令牌                                               | 作用                                                                          |
| -------------------------------------------------- | ----------------------------------------------------------------------------- |
| `codeBackground` / `codeForeground` / `codeBorder` | 代码块与行内代码的背景、文字、边框。                                          |
| `codeKeyword` / `codeBoolean` / `codeNumber`       | 行内代码轻量着色与 JSON 大文件编辑器着色：关键字（含 `null`）、布尔值、数字。 |
| `codeString` / `codeOperator` / `codePunctuation`  | 同上：字符串、运算符、标点。                                                  |
| `tableBorder` / `tableHeaderBackground`            | 表格边框 / 表头背景。                                                         |

已识别语言的代码块由 `shikiTheme` 负责高亮。

### 窗口外框与滚动条

| 令牌                                                           | 作用                                                               |
| -------------------------------------------------------------- | ------------------------------------------------------------------ |
| `titlebarBackground` / `titlebarBorder` / `titlebarForeground` | 标题栏背景、底边线、文字。macOS 程序坞图标也参考标题栏背景的明暗。 |
| `dropdownBackground` / `dropdownBorder`                        | 下拉菜单、浮层背景与边框。                                         |
| `scrollbarTrack`                                               | 滚动条轨道。                                                       |
| `scrollbarThumb`                                               | 滚动条滑块。                                                       |
| `scrollbarThumbIdle`                                           | 预留的滑块空闲色，当前版本未使用，可保持模板原值。                 |
| `scrollbarThumbHover` / `scrollbarThumbActive`                 | 滑块悬停 / 拖动。                                                  |

## 样式令牌

| 令牌                                                     | 取值                         | 作用                                          |
| -------------------------------------------------------- | ---------------------------- | --------------------------------------------- |
| `radiusXs` `radiusSm` `radiusMd` `radiusLg` `radiusPill` | `0` 或带 `px` / `rem` 的数字 | 从小到大的圆角；`radiusPill` 用于胶囊形控件。 |
| `borderWidth`                                            | 同上                         | 控件边框宽度。                                |
| `spaceXs` `spaceSm` `spaceMd` `spaceLg` `spaceXl`        | 同上                         | 从小到大的间距。                              |
| `controlHeightSm` `controlHeightMd` `controlHeightLg`    | 同上                         | 按钮、输入框等控件高度。                      |
| `shadowRaised` `shadowFloating` `shadowDialog`           | CSS `box-shadow` 值          | 卡片、浮层、对话框阴影。                      |
| `fontUi` `fontDocument` `fontMono`                       | CSS `font-family` 字体栈     | 界面、正文、代码字体。                        |

长度类令牌只接受 `0`、`12px`、`0.75rem` 这类写法，不接受 `calc()`、`%`、`em` 等。

## 字体

只想换字体时不需要做主题：在 **偏好设置 → 外观 → 字体** 中分别填写界面、正文、代码字体的名称（如 `LXGW WenKai`、`JetBrains Mono`），回车或离开输入框后生效。

- 每项只填一个字体名，不加引号和逗号；主题原有字体栈会自动保留为回退。
- 输入框会联想本机已安装的常用字体；输入的字体未检测到时会给出提示，并回退到主题字体。
- 也可以填写通用字体族：`serif`、`sans-serif`、`monospace`、`system-ui`、`ui-monospace` 等。
- 字体设置作用于编辑器、导出的 HTML / PDF 和 macOS Quick Look 预览；Word 导出仍使用固定字体。
- 设置里的字体优先于主题 `styleTokens` 中的 `fontUi` / `fontDocument` / `fontMono`。

## 安全与限制

- 令牌值不能包含 `;`、`{`、`}`、换行，也不能使用 `url(`、`@import`、`!important`。
- 未知的令牌名会直接报错（防止拼写错误被静默忽略）。
- 单个文件不超过 256 KB，最多保存 64 个自定义主题。
- 主题只改变颜色与样式，不能注入脚本或任意 CSS。

## 存储位置

导入的主题保存为应用数据目录下的 `themes/<id>.json`：

| 系统    | 路径                                                     |
| ------- | -------------------------------------------------------- |
| macOS   | `~/Library/Application Support/com.nomo.desktop/themes/` |
| Windows | `%APPDATA%\com.nomo.desktop\themes\`                     |
| Linux   | `~/.local/share/com.nomo.desktop/themes/`                |

也可以直接把文件放进该目录，文件名必须等于 `id`（如 `user-ocean.json`）。重新打开设置窗口或重启 Nomo 后加载。

## 常见问题

| 现象                                         | 原因与处理                                                           |
| -------------------------------------------- | -------------------------------------------------------------------- |
| 导入时提示 `id 必须以 user- 开头…`           | `id` 不符合命名规则，或直接用了内置主题的 `id`。                     |
| 提示 `未知令牌：variants.light.tokens.x`     | 令牌名拼错，或写在了错误的分组（颜色写进了 `styleTokens`）。         |
| 提示 `主题令牌值非法` / `主题样式令牌值非法` | 值里含有禁止的字符或写法；长度类令牌只接受 `px` / `rem`。            |
| 提示 `不支持的 Shiki 主题`                   | `shikiTheme` 不在上文列出的四个值中。                                |
| 提示 `JSON 格式错误`                         | 文件不是合法 JSON，常见于多余的逗号或注释。                          |
| 设置页出现“以下主题文件无法加载”             | 主题目录里的文件已损坏或不再兼容，可修正后重新导入，或在列表中删除。 |
| 删除当前主题后变成默认主题                   | 预期行为：正在使用的主题被删除时自动切回 Nomo 默认。                 |

## 让 AI 助手生成主题

先导出一个内置主题作为模板，然后把模板和下面的提示一起发给 AI 助手：

```text
这是 Nomo 编辑器的主题模板 JSON。请基于它生成一个新主题：
- 风格：<描述你想要的配色，例如“低饱和的森林绿，护眼”>
- id 改为 user-<英文短名>，localizedNames 提供 zh-CN 和 en-US 名称
- 保留所有字段和令牌名，只修改颜色值；light 和 dark 两个变体都要调整
- 保证 foreground 与 documentBackground、onAccent 与 accentFill 的对比度不低于 WCAG AA（4.5:1）
- selection 使用半透明颜色；不要使用 url()、!important、分号或花括号
- mermaid.theme 设为 "base"，shikiTheme 只能从 github-light、github-dark、gruvbox-light-medium、gruvbox-dark-medium 中选择
只输出完整 JSON。
```
