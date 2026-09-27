//! 文件名模板渲染、合法化与重命名预览（设计方案 §9 / R2）。
//!
//! * 语法：`{变量}`，`{{` / `}}` 转义花括号；`{track:02}` 支持零填充。
//! * 变量：`artist title album albumartist track disc year genre comment`。
//! * 缺失字段策略：留空 / 占位符 / 跳过该文件。
//! * 合法化：Windows 非法字符、结尾空格与点、多余空格、全半角、保留名、长度限制。
//! * 预览与冲突检测全部在内存中完成（源文件只读，非破坏性）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{TagField, Tags};

/// 模板可用变量。
pub const VARIABLES: [&str; 9] = [
    "artist",
    "title",
    "album",
    "albumartist",
    "track",
    "disc",
    "year",
    "genre",
    "comment",
];

/// 内置模板预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplatePreset {
    /// 稳定 id。
    pub id: &'static str,
    /// 中文标签。
    pub label: &'static str,
    /// 模板内容。
    pub template: &'static str,
}

/// 全部内置预设（顺序即 UI 下拉顺序）。
pub fn presets() -> &'static [TemplatePreset] {
    &[
        TemplatePreset {
            id: "artist-title",
            label: "艺术家 - 标题",
            template: "{artist} - {title}",
        },
        TemplatePreset {
            id: "title-artist",
            label: "标题 - 艺术家",
            template: "{title} - {artist}",
        },
        TemplatePreset {
            id: "track-artist-title",
            label: "音轨号 艺术家 - 标题",
            template: "{track:02} {artist} - {title}",
        },
        TemplatePreset {
            id: "track-title",
            label: "音轨号 - 标题",
            template: "{track:02} - {title}",
        },
        TemplatePreset {
            id: "disc-track-title",
            label: "碟号-音轨号 - 标题",
            template: "{disc}-{track:02} - {title}",
        },
        TemplatePreset {
            id: "album-track-title",
            label: "专辑 - 音轨号 - 标题",
            template: "{album} - {track:02} - {title}",
        },
        TemplatePreset {
            id: "artist-album-track-title",
            label: "艺术家 - 专辑 - 音轨号 - 标题",
            template: "{artist} - {album} - {track:02} - {title}",
        },
        TemplatePreset {
            id: "title",
            label: "仅标题",
            template: "{title}",
        },
    ]
}

/// 查找预设。
pub fn preset(id: &str) -> Option<TemplatePreset> {
    presets().iter().copied().find(|p| p.id == id)
}

/// 缺失字段处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingFieldPolicy {
    /// 留空（默认）。
    #[default]
    Empty,
    /// 保留 `{变量}` 占位符，便于人工发现。
    Placeholder,
    /// 跳过该文件。
    Skip,
}

/// 合法化选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeOptions {
    /// 非法字符的替换字符。
    pub replacement: char,
    /// 折叠连续空格 / 制表符。
    pub collapse_spaces: bool,
    /// 全角字符转半角。
    pub fullwidth_to_halfwidth: bool,
    /// 文件名主体最大长度（字符数，不含扩展名）。
    pub max_len: usize,
    /// 全部字段缺失时的回退名。
    pub fallback: String,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        SanitizeOptions {
            replacement: '_',
            collapse_spaces: true,
            fullwidth_to_halfwidth: true,
            max_len: 180,
            fallback: "未命名".into(),
        }
    }
}

/// 渲染 + 合法化的完整选项。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NamingOptions {
    /// 缺失字段策略。
    pub missing: MissingFieldPolicy,
    /// 合法化选项。
    pub sanitize: SanitizeOptions,
}

/// 模板渲染结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedName {
    /// 渲染出的原始文本（已按缺失策略处理）。
    pub text: String,
    /// 模板中引用但缺失的变量。
    pub missing_variables: Vec<String>,
    /// 是否因缺失字段而应跳过。
    pub skip: bool,
}

/// 渲染模板（不做法则化）。
pub fn render(template: &str, tags: &Tags, policy: MissingFieldPolicy) -> RenderedName {
    let tags = tags.clone().normalized();
    let mut out = String::with_capacity(template.len() * 2);
    let mut missing = Vec::new();
    let mut skip = false;

    let chars: Vec<char> = template.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                if chars.get(i + 1) == Some(&'{') {
                    out.push('{');
                    i += 2;
                    continue;
                }
                // 解析变量名
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len() && chars[j] != '}' {
                    name.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    // 未闭合：原样输出
                    out.push('{');
                    i += 1;
                    continue;
                }
                let (var, pad) = parse_variable(&name);
                match resolve(var, &tags, pad) {
                    Some(value) => out.push_str(&value),
                    None => {
                        missing.push(var.to_string());
                        match policy {
                            MissingFieldPolicy::Empty => {}
                            MissingFieldPolicy::Placeholder => {
                                out.push('{');
                                out.push_str(&name);
                                out.push('}');
                            }
                            MissingFieldPolicy::Skip => {
                                skip = true;
                            }
                        }
                    }
                }
                i = j + 1;
            }
            '}' => {
                if chars.get(i + 1) == Some(&'}') {
                    i += 2;
                } else {
                    i += 1;
                }
                out.push('}');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    RenderedName {
        text: out,
        missing_variables: missing,
        skip,
    }
}

/// 解析 `name`（可能是 `track` 或 `track:02`），返回 (变量名, 零填充宽度)。
fn parse_variable(name: &str) -> (&str, Option<usize>) {
    match name.split_once(':') {
        Some((var, spec)) => {
            let spec = spec.trim();
            // `{track:02}` → 宽度 2；`{track:3}`(或 `03`) → 宽度 3
            let width = if !spec.is_empty() && spec.chars().all(|c| c.is_ascii_digit()) {
                let value = spec.parse::<usize>().unwrap_or(0);
                Some(value.max(spec.len()).min(8))
            } else {
                None
            };
            (var.trim(), width)
        }
        None => (name, None),
    }
}

fn resolve(var: &str, tags: &Tags, pad: Option<usize>) -> Option<String> {
    let raw = match var {
        "title" => tags.title.clone(),
        "artist" => tags.artist.clone(),
        "album" => tags.album.clone(),
        "albumartist" => tags.album_artist.clone(),
        "track" => tags.track.map(|v| match pad {
            Some(w) => format!("{v:0w$}"),
            None => v.to_string(),
        }),
        "disc" => tags.disc.map(|v| match pad {
            Some(w) => format!("{v:0w$}"),
            None => v.to_string(),
        }),
        "year" => tags.year.clone(),
        "genre" => tags.genre.clone(),
        "comment" => tags.comment.clone(),
        _ => None,
    };
    raw.filter(|v| !v.trim().is_empty())
}

/// 合法化结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sanitized {
    /// 合法化后的文件名主体。
    pub text: String,
    /// 是否发生了改动。
    pub changed: bool,
    /// 修改说明（UI 展示提示）。
    pub notes: Vec<String>,
}

/// Windows 保留设备名。
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Windows 非法字符。
pub const ILLEGAL_CHARS: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// 合法化一个文件名主体（不含扩展名）。
pub fn sanitize(name: &str, opts: &SanitizeOptions) -> Sanitized {
    let original = name.to_string();
    let mut notes: Vec<String> = Vec::new();
    let mut text = name.to_string();

    if opts.fullwidth_to_halfwidth {
        let converted = to_halfwidth(&text);
        if converted != text {
            notes.push("全角字符已转半角".into());
            text = converted;
        }
    }

    // 控制字符与非法字符
    let mut replaced = String::with_capacity(text.len());
    let mut hit_illegal = false;
    for c in text.chars() {
        if c == '\t' || c == '\n' || c == '\r' || c == '\u{0b}' || c == '\u{0c}' {
            // 制表符/换行等空白类控制字符按空格处理
            replaced.push(' ');
        } else if c.is_control() || ILLEGAL_CHARS.contains(&c) {
            hit_illegal = true;
            replaced.push(opts.replacement);
        } else {
            replaced.push(c);
        }
    }
    if hit_illegal {
        notes.push(format!(
            "非法字符已替换为 “{}”（Windows 不允许 \\ / : * ? \" < > |）",
            opts.replacement
        ));
    }
    text = replaced;

    if opts.collapse_spaces {
        let collapsed = collapse_whitespace(&text);
        if collapsed != text {
            notes.push("多余空格已合并".into());
            text = collapsed;
        }
    }

    // 首尾空格与结尾的点
    let trimmed = text.trim().trim_end_matches(['.', ' ']).trim().to_string();
    if trimmed != text {
        notes.push("去除了首尾空格或结尾的点".into());
        text = trimmed;
    }

    // 保留设备名
    let stem_upper = text
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem_upper.as_str()) {
        notes.push(format!("“{text}” 是 Windows 保留名，已加后缀"));
        text = format!("{text}_");
    }

    // 长度限制（按字符数截断）
    if text.chars().count() > opts.max_len {
        notes.push(format!("文件名超过 {} 字符已截断", opts.max_len));
        text = text.chars().take(opts.max_len).collect::<String>();
        text = text.trim_end_matches(['.', ' ']).to_string();
    }

    if text.is_empty() {
        notes.push(format!("名称为空，已使用回退名 “{}”", opts.fallback));
        text = opts.fallback.clone();
    }

    Sanitized {
        changed: text != original,
        text,
        notes,
    }
}

/// 全角 → 半角（ASCII 区与全角空格）。
pub fn to_halfwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        let is_space = c.is_whitespace();
        if is_space {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// 重命名预览的输入项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenameInput {
    /// 源文件路径。
    pub path: PathBuf,
    /// 源文件名（含扩展名）。
    pub file_name: String,
    /// 标签（模板变量来源）。
    pub tags: Tags,
}

/// 冲突类型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenameConflict {
    /// 批内重名。
    DuplicateInBatch {
        /// 与之冲突的文件名。
        name: String,
    },
    /// 输出目录已存在同名文件。
    ExistsOnDisk {
        /// 目标路径。
        path: PathBuf,
    },
    /// 名称与源文件相同（无需处理）。
    Unchanged,
}

impl RenameConflict {
    /// 中文说明。
    pub fn note(&self) -> String {
        match self {
            RenameConflict::DuplicateInBatch { name } => format!("与批内文件重名：{name}"),
            RenameConflict::ExistsOnDisk { .. } => "输出目录已存在同名文件".into(),
            RenameConflict::Unchanged => "名称未变化".into(),
        }
    }
}

/// 单条重命名预览。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePreviewItem {
    /// 源路径。
    pub source_path: PathBuf,
    /// 原名。
    pub original_name: String,
    /// 新名（含扩展名）。
    pub new_name: String,
    /// 是否发生变化。
    pub changed: bool,
    /// 是否应跳过（缺失字段策略 = 跳过）。
    pub skip: bool,
    /// 提示信息。
    pub notes: Vec<String>,
    /// 冲突信息。
    pub conflict: Option<RenameConflict>,
}

/// 预览重命名结果。
///
/// `exists` 用于判断输出目录中是否已存在同名文件（单测可注入内存实现，避免真实 I/O）。
pub fn preview_rename(
    inputs: &[RenameInput],
    template: &str,
    options: &NamingOptions,
    output_dir: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<RenamePreviewItem> {
    let mut items: Vec<RenamePreviewItem> = Vec::with_capacity(inputs.len());
    let mut seen: Vec<(String, String)> = Vec::new(); // (new_name_lowercase, original_name)

    for input in inputs {
        let rendered = render(template, &input.tags, options.missing);
        let sanitized = sanitize(&rendered.text, &options.sanitize);

        let ext = Path::new(&input.file_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let new_name = format!("{}{}", sanitized.text, ext);

        let mut notes = sanitized.notes.clone();
        for v in &rendered.missing_variables {
            notes.push(format!("缺少字段 {{{v}}}"));
        }

        let mut conflict = None;
        if !rendered.skip {
            let key = new_name.to_lowercase();
            if let Some((_, other)) = seen.iter().find(|(k, _)| *k == key) {
                conflict = Some(RenameConflict::DuplicateInBatch {
                    name: other.clone(),
                });
            } else {
                seen.push((key, input.file_name.clone()));
                let target = output_dir.join(&new_name);
                if new_name == input.file_name {
                    conflict = Some(RenameConflict::Unchanged);
                } else if exists(&target) {
                    conflict = Some(RenameConflict::ExistsOnDisk { path: target });
                }
            }
        }

        items.push(RenamePreviewItem {
            source_path: input.path.clone(),
            original_name: input.file_name.clone(),
            changed: !rendered.skip && new_name != input.file_name,
            new_name,
            skip: rendered.skip,
            notes,
            conflict,
        });
    }

    items
}

/// 批量查找替换（标签页使用）。
pub fn replace_in_field(
    tags: &mut Tags,
    field: TagField,
    find: &str,
    replace: &str,
    case_insensitive: bool,
) -> bool {
    if find.is_empty() {
        return false;
    }
    let Some(current) = field.get(tags) else {
        return false;
    };
    let (haystack, needle) = if case_insensitive {
        (current.to_lowercase(), find.to_lowercase())
    } else {
        (current.clone(), find.to_string())
    };
    if !haystack.contains(&needle) {
        return false;
    }
    let new_value = if case_insensitive {
        replace_all_case_insensitive(&current, find, replace)
    } else {
        current.replace(find, replace)
    };
    field.set(tags, Some(new_value)).is_ok()
}

fn replace_all_case_insensitive(haystack: &str, find: &str, replace: &str) -> String {
    let lower = haystack.to_lowercase();
    let needle = find.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut pos = 0usize;
    while let Some(idx) = lower[pos..].find(&needle) {
        let start = pos + idx;
        out.push_str(&haystack[pos..start]);
        out.push_str(replace);
        pos = start + needle.len();
    }
    out.push_str(&haystack[pos..]);
    out
}

/// 从文件名反推标签（模板的逆操作，支持 `{artist} - {title}` 之类的常见分隔）。
pub fn guess_tags_from_filename(file_name: &str, template: &str) -> Tags {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name)
        .to_string();

    // 把模板切成 文本片段 / 变量 序列
    let variables = template_variables(template);
    if variables.is_empty() {
        return Tags::default();
    }

    let parts = split_by_template(template);
    // 预校验：所有非空字面量必须按顺序出现，否则视为模板不匹配
    {
        let mut cursor = 0usize;
        for part in &parts {
            if let TemplatePart::Literal(text) = part {
                if text.is_empty() {
                    continue;
                }
                match stem[cursor.min(stem.len())..].find(text.as_str()) {
                    Some(found) => cursor += found + text.len(),
                    None => return Tags::default(),
                }
            }
        }
    }

    let mut tags = Tags::default();
    let mut cursor = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        match part {
            TemplatePart::Literal(text) => {
                if text.is_empty() {
                    continue;
                }
                if let Some(found) = stem[cursor.min(stem.len())..].find(text.as_str()) {
                    cursor += found + text.len();
                } else if idx == 0 {
                    // 前缀不匹配，放弃
                    return Tags::default();
                }
            }
            TemplatePart::Variable { name, .. } => {
                let next_literal = parts[idx + 1..].iter().find_map(|p| match p {
                    TemplatePart::Literal(t) if !t.is_empty() => Some(t.clone()),
                    _ => None,
                });
                let rest = &stem[cursor.min(stem.len())..];
                let value = match next_literal {
                    Some(lit) => match rest.find(lit.as_str()) {
                        Some(pos) => {
                            let v = rest[..pos].to_string();
                            cursor += pos;
                            v
                        }
                        None => {
                            let v = rest.to_string();
                            cursor = stem.len();
                            v
                        }
                    },
                    None => {
                        let v = rest.to_string();
                        cursor = stem.len();
                        v
                    }
                };
                assign_variable(&mut tags, name, value.trim());
            }
        }
    }
    tags.normalized()
}

#[derive(Debug, Clone, PartialEq)]
enum TemplatePart {
    Literal(String),
    Variable { name: String, raw: String },
}

fn split_by_template(template: &str) -> Vec<TemplatePart> {
    let chars: Vec<char> = template.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                literal.push('{');
                i += 2;
            }
            '{' => {
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len() && chars[j] != '}' {
                    name.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    literal.push('{');
                    i += 1;
                } else {
                    if !literal.is_empty() {
                        parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(TemplatePart::Variable {
                        name: name.clone(),
                        raw: name,
                    });
                    i = j + 1;
                }
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                literal.push('}');
                i += 2;
            }
            c => {
                literal.push(c);
                i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(TemplatePart::Literal(literal));
    }
    parts
}

/// 模板中出现的变量名（去重、保持顺序）。
pub fn template_variables(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in split_by_template(template) {
        if let TemplatePart::Variable { name, .. } = part {
            let (var, _) = parse_variable(&name);
            if VARIABLES.contains(&var) && !out.iter().any(|v| v == var) {
                out.push(var.to_string());
            }
        }
    }
    out
}

fn assign_variable(tags: &mut Tags, name: &str, value: &str) {
    let (var, _) = parse_variable(name);
    let numeric = || value.parse::<u32>().ok();
    match var {
        "title" => tags.title = Some(value.to_string()),
        "artist" => tags.artist = Some(value.to_string()),
        "album" => tags.album = Some(value.to_string()),
        "albumartist" => tags.album_artist = Some(value.to_string()),
        "track" => tags.track = numeric(),
        "disc" => tags.disc = numeric(),
        "year" => tags.year = Some(value.to_string()),
        "genre" => tags.genre = Some(value.to_string()),
        "comment" => tags.comment = Some(value.to_string()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Tags {
        Tags {
            title: Some("Yesterday".into()),
            artist: Some("The Beatles".into()),
            album: Some("Help!".into()),
            album_artist: Some("The Beatles".into()),
            track: Some(7),
            disc: Some(1),
            year: Some("1965".into()),
            genre: Some("Rock/Pop".into()),
            comment: None,
        }
    }

    #[test]
    fn default_preset_renders_artist_and_title() {
        let r = render("{artist} - {title}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "The Beatles - Yesterday");
        assert!(r.missing_variables.is_empty());
        assert!(!r.skip);
        assert_eq!(presets()[0].template, "{artist} - {title}");
    }

    #[test]
    fn preset_list_offers_title_first_ordering() {
        // 用户可在“艺术家 - 标题”与“标题 - 艺术家”之间直接切换
        let preset = preset("title-artist").expect("应内置 {title} - {artist} 预设");
        assert_eq!(preset.template, "{title} - {artist}");
        assert_eq!(preset.label, "标题 - 艺术家");
        let rendered = render(preset.template, &tags(), MissingFieldPolicy::Empty);
        assert_eq!(rendered.text, "Yesterday - The Beatles");
        assert_eq!(
            presets()[1].id,
            "title-artist",
            "应紧跟在“艺术家 - 标题”之后"
        );
    }

    #[test]
    fn escaped_braces_are_literal() {
        let r = render("{{{title}}}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "{Yesterday}");
    }

    #[test]
    fn zero_padding_applies_to_numbers() {
        let r = render("{track:02} - {title}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "07 - Yesterday");
        let r = render("{track:03}_{disc:02}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "007_01");
        let r = render("{track}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "7");
    }

    #[test]
    fn missing_field_policies_behave_differently() {
        let t = Tags {
            title: Some("Song".into()),
            ..Tags::default()
        };
        let empty = render("{artist} - {title}", &t, MissingFieldPolicy::Empty);
        assert_eq!(empty.text, " - Song");
        assert_eq!(empty.missing_variables, vec!["artist"]);

        let placeholder = render("{artist} - {title}", &t, MissingFieldPolicy::Placeholder);
        assert_eq!(placeholder.text, "{artist} - Song");

        let skip = render("{artist} - {title}", &t, MissingFieldPolicy::Skip);
        assert!(skip.skip);
    }

    #[test]
    fn unknown_variables_are_treated_as_missing() {
        let r = render("{bogus}-{title}", &tags(), MissingFieldPolicy::Empty);
        assert_eq!(r.text, "-Yesterday");
        assert_eq!(template_variables("{bogus}-{title}"), vec!["title"]);
        assert_eq!(
            template_variables("{artist}-{title}-{artist}"),
            vec!["artist", "title"]
        );
    }

    #[test]
    fn sanitize_handles_illegal_chars_and_edges() {
        let opts = SanitizeOptions::default();
        let s = sanitize("AC\\DC: Back? \"in\" <black>|", &opts);
        assert_eq!(s.text, "AC_DC_ Back_ _in_ _black__");
        assert!(s.changed);
        assert!(!s.notes.is_empty());

        let s = sanitize("  name...  ", &opts);
        assert_eq!(s.text, "name");

        let s = sanitize("a    b\tc", &opts);
        assert_eq!(s.text, "a b c");
    }

    #[test]
    fn sanitize_converts_fullwidth_and_protects_reserved_names() {
        let opts = SanitizeOptions::default();
        assert_eq!(sanitize("全角ＡＢＣ１２３", &opts).text, "全角ABC123");
        assert_eq!(sanitize("CON", &opts).text, "CON_");
        assert_eq!(sanitize("nul", &opts).text, "nul_");
        assert_eq!(sanitize("", &opts).text, "未命名");
        assert_eq!(sanitize("云　南", &opts).text, "云 南");
    }

    #[test]
    fn sanitize_truncates_long_names() {
        let opts = SanitizeOptions {
            max_len: 10,
            ..SanitizeOptions::default()
        };
        let s = sanitize("abcdefghijklmnop", &opts);
        assert_eq!(s.text, "abcdefghij");
        assert!(s.notes.iter().any(|n| n.contains("截断")));
    }

    #[test]
    fn preview_detects_all_conflict_kinds() {
        let inputs = vec![
            RenameInput {
                path: PathBuf::from("C:/in/a.flac"),
                file_name: "a.flac".into(),
                tags: tags(),
            },
            RenameInput {
                path: PathBuf::from("C:/in/b.flac"),
                file_name: "b.flac".into(),
                tags: tags(), // 与 a 的标签相同 → 批内重名
            },
            RenameInput {
                path: PathBuf::from("C:/in/The Beatles - Yesterday.flac"),
                file_name: "The Beatles - Yesterday.flac".into(),
                tags: tags(), // 名称未变化，但批内已被 a 占用 → 批内重名
            },
            RenameInput {
                path: PathBuf::from("C:/in/c.flac"),
                file_name: "c.flac".into(),
                tags: Tags {
                    title: Some("Exists".into()),
                    artist: Some("X".into()),
                    ..Tags::default()
                },
            },
            RenameInput {
                path: PathBuf::from("C:/in/d.flac"),
                file_name: "d.flac".into(),
                tags: Tags {
                    title: Some("NoArtist".into()),
                    ..Tags::default()
                },
            },
        ];
        let out_dir = PathBuf::from("C:/out");
        let exists = |p: &Path| p.to_string_lossy().ends_with("X - Exists.flac");
        let items = preview_rename(
            &inputs,
            "{artist} - {title}",
            &NamingOptions::default(),
            &out_dir,
            &exists,
        );
        assert_eq!(items[0].new_name, "The Beatles - Yesterday.flac");
        assert!(items[0].changed);
        assert_eq!(items[0].conflict, None);
        assert!(matches!(
            items[1].conflict,
            Some(RenameConflict::DuplicateInBatch { .. })
        ));
        assert!(matches!(
            items[3].conflict,
            Some(RenameConflict::ExistsOnDisk { .. })
        ));
        assert_eq!(items[4].new_name, "- NoArtist.flac");
        assert!(items[4].notes.iter().any(|n| n.contains("缺少字段")));
    }

    #[test]
    fn preview_honours_skip_policy() {
        let inputs = vec![RenameInput {
            path: PathBuf::from("in.flac"),
            file_name: "in.flac".into(),
            tags: Tags::default(),
        }];
        let opts = NamingOptions {
            missing: MissingFieldPolicy::Skip,
            ..NamingOptions::default()
        };
        let items = preview_rename(
            &inputs,
            "{artist} - {title}",
            &opts,
            Path::new("C:/out"),
            &|_| false,
        );
        assert!(items[0].skip);
        assert!(!items[0].changed);
    }

    #[test]
    fn batch_replace_works_case_insensitively() {
        let mut t = tags();
        assert!(replace_in_field(
            &mut t,
            TagField::Artist,
            "beatles",
            "Beatles",
            true
        ));
        assert_eq!(t.artist.as_deref(), Some("The Beatles"));
        assert!(!replace_in_field(
            &mut t,
            TagField::Artist,
            "zzz",
            "x",
            true
        ));
        assert!(!replace_in_field(&mut t, TagField::Artist, "", "x", false));
        let mut t2 = tags();
        assert!(replace_in_field(&mut t2, TagField::Comment, "a", "b", false) == false);
    }

    #[test]
    fn guess_tags_from_filename_reverses_common_templates() {
        let t = guess_tags_from_filename("The Beatles - Yesterday.flac", "{artist} - {title}");
        assert_eq!(t.artist.as_deref(), Some("The Beatles"));
        assert_eq!(t.title.as_deref(), Some("Yesterday"));

        let t = guess_tags_from_filename(
            "07 The Beatles - Yesterday.mp3",
            "{track:02} {artist} - {title}",
        );
        assert_eq!(t.track, Some(7));
        assert_eq!(t.artist.as_deref(), Some("The Beatles"));
        assert_eq!(t.title.as_deref(), Some("Yesterday"));

        let t = guess_tags_from_filename("NoSeparator.flac", "{artist} - {title}");
        assert_eq!(t.artist, None);
        assert_eq!(t.title, None);

        let t = guess_tags_from_filename("JustATitle.flac", "{title}");
        assert_eq!(t.title.as_deref(), Some("JustATitle"));
    }

    #[test]
    fn preset_lookup() {
        assert!(preset("track-title").is_some());
        assert!(preset("nope").is_none());
    }
}
