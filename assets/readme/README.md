# 首版候选视觉资源

这些文件是本次 README 设计首版生成的候选素材，目前不再由中英文 README 引用。当前首屏采用 `assets/128x128.png` 小图标与 Nomo 标题，并恢复 `assets/nomo-editor-zh.gif`、`assets/nomo-editor-en.gif`；下文仅保留首版素材的来源记录。

面向本地 Markdown 写作者，主叙事为「本地写作，轻快而从容」。首屏展示真实原生界面，随后说明写作特色、性能机制与有范围的测量。第一步是下载已发布版本，或按构建指南体验原生开发分支。

- 配色：沿用 Nomo 默认主题的浅灰 `#f6f7f9`、正文 `#202428`、低饱和绿 `#2f7d6f`；图标蓝色光标保持既有应用标识。
- 字体：系统 UI 字体；大标题与正文分级，短元信息采用克制的字距。
- 形状：20px 横幅圆角、2px 示意线条、8px 间距基准；不使用外部字体、SVG 脚本、滤镜或动画。
- 图形：应用 N 标记、写作光标和 `.md` 文件，均为静态矢量元素。横幅是品牌插画，不是应用截图；长文案与性能数据保留为可搜索的 Markdown。

`hero-light.svg` / `hero-dark.svg` 共用 1200×300 画布，应用标记取自 [`nomo-app-light.svg`](../../src-gpui/assets/nomo-app-light.svg) 与 [`nomo-app-dark.svg`](../../src-gpui/assets/nomo-app-dark.svg)。首版候选曾使用 `<picture>` 根据页面主题切换，当前两份 README 不引用这些横幅。

`editor-zh-light.png`、`editor-zh-dark.png`、`editor-en-light.png` 为 2026-11-07 经用户授权拍摄的原生 Windows 窗口，1282×821 像素。内容来自本次隔离演示文档，不含用户日常文档；截图保留原始像素与工具产生的鼠标标记，未重绘界面或补造内容。英文图使用英文正文、中文界面。表格底部在视口外，属正常文档滚动裁切。

截图构建为现有 Release automation 副本，SHA-256 `3e2b199dc6ec5b3dbbc97b0be30cb19abcf7d2d14cd23bc52f0536edd040fd50`。截图只展示该构建的界面，不能用于证明动态流畅度、性能、跨平台一致性或发布版功能。隔离截图实例已正常退出。

更新时重新核对真实界面，保留有意义的 alt，并检查 900px 与 360px 宽度下的排版；在窄屏上，主要说明应从图片旁的 Markdown 阅读。
