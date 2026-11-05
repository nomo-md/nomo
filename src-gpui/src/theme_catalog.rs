//! 原版 schemaVersion=1 主题目录。只解析声明式令牌，不执行 CSS 或加载外部资源。
//! 目录最多 32 个用户主题，单文件 128 KiB，规范化后的主题总量最多 2 MiB。
use super::Theme;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
use crate::{file_system::write_file_atomically, settings::Settings};
use gpui::{App, Global, Hsla, PathPromptOptions, WindowAppearance};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, path::{Path, PathBuf}, sync::Arc};

const MAX_FILE_BYTES: usize = 128 * 1024;
const MAX_USER_THEMES: usize = 32;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColorTheme {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub author: String,
    pub localized_names: BTreeMap<String, String>,
    pub style_profile: String,
    pub variants: BTreeMap<String, Variant>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Variant {
    pub tokens: BTreeMap<String, String>,
    pub style_tokens: BTreeMap<String, String>,
    pub shiki_theme: String,
    pub mermaid: Mermaid,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Mermaid { pub theme: String }

impl ColorTheme {
    pub fn name(&self, locale: &str) -> &str {
        self.localized_names.get(locale).or_else(|| self.localized_names.get("en-US"))
            .map(String::as_str).unwrap_or(&self.id)
    }
    pub fn variant(&self, dark: bool) -> &Variant { &self.variants[if dark { "dark" } else { "light" }] }
    pub fn builtin(&self) -> bool { !self.id.starts_with("user-") }
    fn bytes(&self) -> usize { serde_json::to_vec(self).map_or(MAX_TOTAL_BYTES, |data| data.len()) }
}

impl Variant {
    pub fn color(&self, key: &str) -> Hsla { parse_color(&self.tokens[key]).expect("已校验主题颜色") }
    pub fn length(&self, key: &str) -> f32 { parse_length(&self.style_tokens[key]).expect("已校验主题长度") }
    pub fn shadow(&self, key: &str) -> gpui::BoxShadow { parse_shadow(&self.style_tokens[key]).expect("已校验主题阴影") }
}

pub(crate) struct ThemeCatalog {
    pub themes: Vec<Arc<ColorTheme>>,
    pub busy: bool,
    pub message: Vec<UiMessage>,
}
impl Global for ThemeCatalog {}
impl ThemeCatalog {
    pub fn selected(&self, settings: &Settings) -> &ColorTheme {
        let id = settings.string("colorThemeId");
        self.themes.iter().find(|theme| theme.id == id).unwrap_or(&self.themes[0])
    }
}

pub(crate) fn init(cx: &mut App) {
    let builtin: Vec<ColorTheme> = serde_json::from_str(include_str!("../resources/color-themes.json")).expect("内置主题定义");
    let themes: Vec<_> = builtin.into_iter().map(Arc::new).collect();
    cx.set_global(ThemeCatalog { themes: themes.clone(), busy: true, message: Vec::new() });
    let task = cx.background_executor().spawn(async move { load_users(&themes) });
    cx.spawn(async move |cx| {
        let (themes, errors) = task.await;
        let _ = cx.update(|cx| {
            let requested = cx.global::<Settings>().string("colorThemeId");
            let catalog = cx.global_mut::<ThemeCatalog>();
            catalog.themes.extend(themes);
            catalog.busy = false;
            let mut errors = errors;
            if !catalog.themes.iter().any(|theme| theme.id == requested) { errors.push(UiMessage::new("theme.selectedMissing").into()); }
            catalog.message = errors;
            for message in &catalog.message { eprintln!("加载主题：{message}"); }
            reapply(cx);
        });
    }).detach();
}

pub(crate) fn reapply(cx: &mut App) {
    // 设置回调仍借用当前窗口，延后读取外观，所有窗口共享同一份目标令牌。
    cx.defer(|cx| {
        let appearance = cx.windows().first().and_then(|window| window.update(cx, |_, window, _| window.appearance()).ok())
            .unwrap_or(WindowAppearance::Light);
        Theme::apply(appearance, cx);
        cx.refresh_windows();
    });
}

pub(crate) fn import(cx: &mut App) {
    if cx.global::<ThemeCatalog>().busy { return; }
    let themes = cx.global::<ThemeCatalog>().themes.clone();
    cx.global_mut::<ThemeCatalog>().busy = true;
    cx.global_mut::<ThemeCatalog>().message.clear();
    let response = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(ToolbarLocale::current(cx).app_text("theme.importTitle").into()) });
    cx.spawn(async move |cx| {
        let result = match response.await {
            Ok(Ok(Some(paths))) => if let Some(path) = paths.into_iter().next() {
                cx.background_executor().spawn(async move {
                    let theme = parse_user(&read_source(&path)?, &themes)?;
                    check_capacity(&themes, &theme)?;
                    let directory = theme_directory()?;
                    std::fs::create_dir_all(&directory).map_err(UiMessage::raw)?;
                    let source = serde_json::to_vec_pretty(&theme).map_err(UiMessage::raw)?;
                    write_file_atomically(&directory.join(format!("{}.json", theme.id)), &source)?;
                    Ok::<_, UiMessage>(Some(theme))
                }).await
            } else { Ok(None) },
            Ok(Ok(None)) => Ok(None),
            Ok(Err(error)) => Err(UiMessage::raw(error)),
            Err(error) => Err(UiMessage::raw(error)),
        };
        let _ = cx.update(|cx| {
            cx.global_mut::<ThemeCatalog>().busy = false;
            match result {
                Ok(Some(theme)) => {
                    let id = theme.id.clone();
                    let catalog = cx.global_mut::<ThemeCatalog>();
                    catalog.themes.retain(|existing| existing.id != id);
                    catalog.themes.push(Arc::new(theme));
                    catalog.message = vec![UiMessage::new("theme.imported")];
                    Settings::set(cx, "colorThemeId", json!(id));
                    // 同 ID 重新导入也需要刷新目标令牌。
                    reapply(cx);
                }
                Ok(None) => {}
                Err(error) => cx.global_mut::<ThemeCatalog>().message = vec![error.context("theme.importFailed")],
            }
            cx.refresh_windows();
        });
    }).detach();
    cx.refresh_windows();
}

pub(crate) fn export(cx: &mut App) {
    if cx.global::<ThemeCatalog>().busy { return; }
    let mut theme = cx.global::<ThemeCatalog>().selected(cx.global::<Settings>()).clone();
    if theme.builtin() {
        theme.id = format!("user-{}-custom", theme.id.trim_start_matches("nomo-"));
        theme.author = "User".into();
        for (locale, name) in &mut theme.localized_names {
            name.push_str(match locale.as_str() { "zh-CN" => "（自定义）", "zh-TW" => "（自訂）", "ja-JP" => "（カスタム）", _ => " Custom" });
        }
    }
    let source = match serde_json::to_vec_pretty(&theme) { Ok(source) => source, Err(error) => {
        cx.global_mut::<ThemeCatalog>().message = vec![UiMessage::raw(error).context("theme.exportFailed")]; return;
    }};
    let directory = crate::settings::config_path().and_then(|path| path.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
    let response = cx.prompt_for_new_path(&directory, Some(&format!("{}.json", theme.id)));
    cx.global_mut::<ThemeCatalog>().busy = true;
    cx.global_mut::<ThemeCatalog>().message.clear();
    cx.spawn(async move |cx| {
        let result = match response.await {
            Ok(Ok(Some(path))) => cx.background_executor().spawn(async move { write_file_atomically(&path, &source).map(|()| true).map_err(UiMessage::raw) }).await,
            Ok(Ok(None)) => Ok(false),
            Ok(Err(error)) => Err(UiMessage::raw(error)), Err(error) => Err(UiMessage::raw(error)),
        };
        let _ = cx.update(|cx| {
            let catalog = cx.global_mut::<ThemeCatalog>();
            catalog.busy = false;
            catalog.message = match result { Ok(true) => vec![UiMessage::new("theme.exported")], Ok(false) => Vec::new(), Err(error) => vec![error.context("theme.exportFailed")] };
            cx.refresh_windows();
        });
    }).detach();
    cx.refresh_windows();
}

fn theme_directory() -> Result<PathBuf, UiMessage> {
    crate::settings::config_path().and_then(|path| path.parent().map(|parent| parent.join("themes"))).ok_or_else(|| UiMessage::new("theme.directory").into())
}
fn read_source(path: &Path) -> Result<String, UiMessage> {
    let file = std::fs::File::open(path).map_err(UiMessage::raw)?;
    if !file.metadata().map_err(UiMessage::raw)?.is_file() { return Err(UiMessage::new("theme.regularFile").into()); }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(UiMessage::raw)?;
    if bytes.len() > MAX_FILE_BYTES { return Err(UiMessage::new("theme.fileBudget").into()); }
    String::from_utf8(bytes).map(|text| text.trim_start_matches('\u{feff}').to_owned()).map_err(|_| UiMessage::new("theme.encoding").into())
}
fn load_users(builtin: &[Arc<ColorTheme>]) -> (Vec<Arc<ColorTheme>>, Vec<UiMessage>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    let entries = match theme_directory().and_then(|path| match std::fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)), Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None), Err(error) => Err(UiMessage::raw(error)),
    }) { Ok(Some(entries)) => entries, Ok(None) => return (loaded, errors), Err(error) => return (loaded, vec![error]) };
    let mut paths = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= 256 { errors.push(UiMessage::new("theme.scanBudget").into()); break; }
        match entry { Ok(entry) if entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")) => paths.push(entry.path()),
            Ok(_) => {}, Err(error) => errors.push(UiMessage::raw(error)) }
    }
    paths.sort();
    let mut all = builtin.to_vec();
    for path in paths {
        let result = read_source(&path).and_then(|source| parse_user(&source, builtin)).and_then(|theme| {
            if path.file_stem().and_then(|name| name.to_str()) != Some(&theme.id) { return Err(UiMessage::new("theme.idFile").into()); }
            check_capacity(&all, &theme)?; Ok(theme)
        });
        match result { Ok(theme) => { let theme = Arc::new(theme); all.push(theme.clone()); loaded.push(theme); },
            Err(error) => errors.push(error.context("theme.pathError").arg("path", path.file_name().unwrap_or_default().to_string_lossy())) }
    }
    (loaded, errors)
}
fn check_capacity(themes: &[Arc<ColorTheme>], theme: &ColorTheme) -> Result<(), UiMessage> {
    let remaining = themes.iter().filter(|entry| !entry.builtin() && entry.id != theme.id);
    if remaining.clone().count() >= MAX_USER_THEMES { return Err(UiMessage::new("theme.countBudget").into()); }
    if remaining.map(|entry| entry.bytes()).sum::<usize>() + theme.bytes() > MAX_TOTAL_BYTES { return Err(UiMessage::new("theme.totalBudget").into()); }
    Ok(())
}

fn parse_user(source: &str, builtin: &[Arc<ColorTheme>]) -> Result<ColorTheme, UiMessage> {
    let raw: Value = serde_json::from_str(source).map_err(|error| UiMessage::new("theme.json").arg("error", error))?;
    let object = raw.as_object().ok_or(UiMessage::new("theme.object"))?;
    if raw["schemaVersion"] != 1 { return Err(UiMessage::new("theme.schema").into()); }
    let id = raw["id"].as_str().ok_or(UiMessage::new("theme.idMissing"))?;
    if id.len() > 64 || !id.starts_with("user-") || !id[5..].split('-').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())) {
        return Err(UiMessage::new("theme.id").into());
    }
    let base_id = raw.get("baseThemeId").map(|value| value.as_str().ok_or(UiMessage::new("theme.baseString"))).transpose()?.unwrap_or("nomo-default");
    let mut theme = builtin.iter().find(|theme| theme.builtin() && theme.id == base_id).ok_or(UiMessage::new("theme.baseBuiltin"))?.as_ref().clone();
    theme.id = id.into();
    theme.author = metadata(raw.get("author"), "User")?;
    theme.version = metadata(raw.get("version"), "1.0.0")?;
    theme.style_profile = metadata(raw.get("styleProfile"), &theme.style_profile)?;
    if !matches!(theme.style_profile.as_str(), "modern" | "paper" | "classic") { return Err(UiMessage::new("theme.profile").into()); }
    let names = raw["localizedNames"].as_object().ok_or(UiMessage::new("theme.namesObject"))?;
    if names.is_empty() || names.len() > 16 { return Err(UiMessage::new("theme.namesCount").into()); }
    theme.localized_names.clear();
    for (locale, value) in names {
        let mut parts = locale.split('-');
        if !parts.next().is_some_and(|part| (2..=3).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || !parts.all(|part| (2..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())) { return Err(UiMessage::new("theme.locale").into()); }
        theme.localized_names.insert(locale.clone(), metadata(Some(value), "")?);
    }
    if let Some(variants) = object.get("variants") {
        for (scheme, value) in variants.as_object().ok_or(UiMessage::new("theme.variantsObject"))? {
            let variant = theme.variants.get_mut(scheme).ok_or(UiMessage::new("theme.schemes"))?;
            let value = value.as_object().ok_or(UiMessage::new("theme.variantObject"))?;
            merge_tokens(&mut variant.tokens, value.get("tokens"), false)?;
            merge_tokens(&mut variant.style_tokens, value.get("styleTokens"), true)?;
            variant.shiki_theme = metadata(value.get("shikiTheme"), &variant.shiki_theme)?;
            if !matches!(variant.shiki_theme.as_str(), "github-light" | "github-dark" | "gruvbox-light-medium" | "gruvbox-dark-medium") { return Err(UiMessage::new("theme.highlight").into()); }
            if let Some(mermaid) = value.get("mermaid") {
                let mermaid = mermaid.as_object().ok_or(UiMessage::new("theme.mermaidObject"))?;
                variant.mermaid.theme = metadata(mermaid.get("theme"), &variant.mermaid.theme)?;
                if !matches!(variant.mermaid.theme.as_str(), "default" | "dark" | "base") { return Err(UiMessage::new("theme.mermaid").into()); }
            }
        }
    }
    Ok(theme)
}
fn metadata(value: Option<&Value>, default: &str) -> Result<String, UiMessage> {
    let text = value.map(|value| value.as_str().ok_or(UiMessage::new("theme.metadataString"))).transpose()?.unwrap_or(default).trim();
    if text.is_empty() || text.chars().count() > 80 || text.chars().any(char::is_control) { return Err(UiMessage::new("theme.metadata").into()); }
    Ok(text.into())
}
fn merge_tokens(base: &mut BTreeMap<String, String>, raw: Option<&Value>, style: bool) -> Result<(), UiMessage> {
    let Some(raw) = raw else { return Ok(()); };
    for (name, value) in raw.as_object().ok_or(UiMessage::new("theme.tokensObject"))? {
        if !base.contains_key(name) { return Err(UiMessage::new("theme.tokenUnknown").arg("name", name)); }
        let value = value.as_str().ok_or_else(|| UiMessage::new("theme.tokenString").arg("name", name))?.trim();
        let safe = value.len() <= 256 && !value.is_empty() && !value.chars().any(|ch| ch.is_control() || matches!(ch, ';' | '{' | '}' | '@' | '!'));
        let valid = if !style { parse_color(value).is_some() }
            else if name.starts_with("font") { font_stack(value).is_some() }
            else if name.starts_with("shadow") { parse_shadow(value).is_some() }
            else { parse_length(value).is_some_and(|length| length <= if name == "radiusPill" { 1000.0 } else { 128.0 }) };
        if !safe || !valid { return Err(UiMessage::new("theme.tokenInvalid").arg("name", name)); }
        base.insert(name.clone(), value.into());
    }
    Ok(())
}

fn parse_length(value: &str) -> Option<f32> {
    let (number, scale) = if let Some(value) = value.strip_suffix("px") { (value, 1.0) }
        else if let Some(value) = value.strip_suffix("rem") { (value, 16.0) } else if value == "0" { (value, 1.0) } else { return None; };
    let value = number.parse::<f32>().ok()? * scale;
    (value.is_finite() && value >= 0.0).then_some(value)
}
pub(super) fn font_stack(value: &str) -> Option<Vec<String>> {
    let fonts: Vec<_> = value.split(',').map(|name| name.trim().trim_matches(['\'', '"']).to_owned()).collect();
    (fonts.len() <= 12 && fonts.iter().all(|name| !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.')))).then_some(fonts)
}
fn parse_shadow(value: &str) -> Option<gpui::BoxShadow> {
    let mut parts = value.splitn(4, ' ');
    let x = parse_length(parts.next()?)?;
    let y = parse_length(parts.next()?)?;
    let blur = parse_length(parts.next()?)?;
    if x > 64.0 || y > 64.0 || blur > 96.0 { return None; }
    Some(gpui::BoxShadow { inset: false, color: parse_color(parts.next()?)?, offset: gpui::point(gpui::px(x), gpui::px(y)), blur_radius: gpui::px(blur), spread_radius: gpui::px(0.0) })
}
fn parse_color(value: &str) -> Option<Hsla> {
    if value == "transparent" { return Some(gpui::rgba(0).into()); }
    if value == "white" { return Some(gpui::rgb(0xffffff).into()); }
    if value == "black" { return Some(gpui::rgb(0).into()); }
    if let Some(hex) = value.strip_prefix('#') {
        let expanded = if matches!(hex.len(), 3 | 4) { hex.chars().flat_map(|ch| [ch, ch]).collect::<String>() } else { hex.into() };
        let number = u32::from_str_radix(&expanded, 16).ok()?;
        return match expanded.len() { 6 => Some(gpui::rgb(number).into()), 8 => Some(gpui::rgba(number).into()), _ => None };
    }
    let (kind, body) = value.split_once('(')?;
    let fields: Vec<_> = body.strip_suffix(')')?.split(',').map(str::trim).collect();
    let alpha = if kind.ends_with('a') { fields.get(3)?.parse::<f32>().ok()? } else { 1.0 };
    if fields.len() != if kind.ends_with('a') { 4 } else { 3 } || !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) { return None; }
    match kind {
        "rgb" | "rgba" => {
            let channel = |index: usize| -> Option<u8> { let value = fields[index].parse::<f32>().ok()?; (value.is_finite() && (0.0..=255.0).contains(&value)).then_some(value.round() as u8) };
            Some(gpui::rgba(((channel(0)? as u32) << 24) | ((channel(1)? as u32) << 16) | ((channel(2)? as u32) << 8) | (alpha * 255.0).round() as u32).into())
        }
        "hsl" | "hsla" => {
            let hue = fields[0].parse::<f32>().ok()?;
            let saturation = fields[1].strip_suffix('%')?.parse::<f32>().ok()?;
            let lightness = fields[2].strip_suffix('%')?.parse::<f32>().ok()?;
            if !hue.is_finite() || !(0.0..=100.0).contains(&saturation) || !(0.0..=100.0).contains(&lightness) { return None; }
            Some(gpui::hsla(hue.rem_euclid(360.0) / 360.0, saturation / 100.0, lightness / 100.0, alpha))
        }
        _ => None,
    }
}
