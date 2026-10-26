//! 所有快捷入口共用命令描述。展示层只负责选择，不直接修改正文。
use super::{commands::EditIntent, parse::InlineFormat};

#[derive(Clone)]
pub struct MenuEntry {
    pub label: String,
    pub keywords: &'static str,
    pub intent: EditIntent,
}
pub fn commands() -> Vec<MenuEntry> {
    let mut result = Vec::new();
    for (label, keywords, format) in [
        ("粗体", "bold", InlineFormat::Strong),
        ("斜体", "italic", InlineFormat::Emphasis),
        ("行内代码", "code", InlineFormat::Code),
        ("删除线", "strike", InlineFormat::Strike),
        ("下划线", "underline", InlineFormat::Underline),
        ("高亮", "highlight", InlineFormat::Highlight),
        ("上标", "superscript", InlineFormat::Superscript),
        ("下标", "subscript", InlineFormat::Subscript),
    ] {
        result.push(MenuEntry {
            label: label.into(),
            keywords,
            intent: EditIntent::ToggleInline(format),
        });
    }
    for (level, label) in [
        (1, "一级标题"),
        (2, "二级标题"),
        (3, "三级标题"),
        (4, "四级标题"),
        (5, "五级标题"),
        (6, "六级标题"),
        (0, "正文"),
    ] {
        result.push(MenuEntry {
            label: label.into(),
            keywords: "heading paragraph",
            intent: EditIntent::Heading(level),
        });
    }
    result.push(MenuEntry {
        label: "清除格式".into(),
        keywords: "clear format",
        intent: EditIntent::ClearFormatting,
    });
    for (label, before) in [("在上方插入段落", true), ("在下方插入段落", false)] {
        result.push(MenuEntry {
            label: label.into(),
            keywords: "paragraph before after",
            intent: EditIntent::InsertParagraph { before },
        });
    }
    result.push(MenuEntry {
        label: "链接".into(),
        keywords: "link",
        intent: EditIntent::Link {
            label: "链接文字".into(),
            target: "https://".into(),
        },
    });
    for (label, keywords, prefix) in [
        ("无序列表", "bullet list", "- "),
        ("有序列表", "ordered list", "1. "),
        ("任务列表", "task checkbox", "- [ ] "),
        ("引用", "quote", "> "),
    ] {
        result.push(MenuEntry {
            label: label.into(),
            keywords,
            intent: EditIntent::LinePrefix(prefix.into()),
        });
    }
    for (label, keywords, template) in [
        ("表格", "table", "| 列一 | 列二 |\n| --- | --- |\n|  |  |"),
        ("代码块", "code block", "```text\n\n```"),
        ("公式块", "math formula", "$$\n\n$$"),
        (
            "Mermaid 图表",
            "mermaid diagram",
            "```mermaid\nflowchart LR\n  A --> B\n```",
        ),
        ("图片", "image", "![图片说明](图片路径)"),
        ("分隔线", "rule divider", "---"),
        (
            "折叠块",
            "details summary",
            "<details open>\n<summary>摘要</summary>\n\n正文\n\n</details>",
        ),
        ("提示", "note callout", "> [!NOTE]\n> 提示内容"),
        ("技巧", "tip callout", "> [!TIP]\n> 技巧内容"),
        ("重要", "important callout", "> [!IMPORTANT]\n> 重要内容"),
        ("警告", "warning callout", "> [!WARNING]\n> 警告内容"),
        ("注意", "caution callout", "> [!CAUTION]\n> 注意内容"),
        ("Front Matter", "yaml metadata", "---\ntitle: 标题\n---"),
        ("脚注", "footnote", "[^note]: 脚注内容"),
        ("注释", "comment", "<!-- 注释 -->"),
        ("目录", "toc contents", "<!-- toc -->\n\n<!-- /toc -->"),
    ] {
        result.push(MenuEntry {
            label: label.into(),
            keywords,
            intent: EditIntent::InsertBlock(template.into()),
        });
    }
    result
}
pub fn search(query: &str) -> Vec<MenuEntry> {
    let query = query.to_lowercase();
    commands()
        .into_iter()
        .filter(|e| e.label.contains(&query) || e.keywords.contains(&query))
        .collect()
}
