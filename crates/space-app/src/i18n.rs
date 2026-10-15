//! Interface language. English strings in the code are the keys; the Russian table
//! maps them to translations. Anything missing falls back to English, and with
//! `SPACE_I18N_REPORT=1` every missing string is printed once so gaps are easy to find.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    /// Name shown in the language menu, always in its own language.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

/// Language of the operating system (the user's locale preference), falling
/// back to the `LC_ALL`, `LC_MESSAGES` and `LANG` environment variables.
pub fn detect() -> Lang {
    if let Some(locale) = sys_locale::get_locale() {
        if let Some(lang) = lang_of_locale(&locale) {
            return lang;
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(lang) = std::env::var(var).ok().as_deref().and_then(lang_of_locale) {
            return lang;
        }
    }
    Lang::En
}

/// `ru-RU`, `ru_RU.UTF-8`, `ru` → Russian; empty or `C` → undecided.
fn lang_of_locale(locale: &str) -> Option<Lang> {
    let locale = locale.trim();
    if locale.is_empty() || locale == "C" || locale == "POSIX" {
        return None;
    }
    let tag = locale
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or(locale);
    Some(if tag.eq_ignore_ascii_case("ru") {
        Lang::Ru
    } else {
        Lang::En
    })
}

/// Size units shown everywhere: `IEC` (KiB, 1024-based, the default) or `SI`
/// (kB, 1000-based, what Finder and disk labels use).
pub use crate::settings::SizeUnits;

static UNITS: AtomicU8 = AtomicU8::new(0);

pub fn set_units(units: SizeUnits) {
    UNITS.store(units as u8, Ordering::Relaxed);
}

pub fn units() -> SizeUnits {
    match UNITS.load(Ordering::Relaxed) {
        1 => SizeUnits::Si,
        _ => SizeUnits::Iec,
    }
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| RU.iter().copied().collect())
}

fn report_missing(s: &str) {
    static SEEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("SPACE_I18N_REPORT").is_some()) {
        return;
    }
    let seen = SEEN.get_or_init(|| Mutex::new(BTreeSet::new()));
    if let Ok(mut seen) = seen.lock() {
        if seen.insert(s.to_owned()) {
            eprintln!("i18n: missing {s:?}");
        }
    }
}

/// Translate `s`; returns it unchanged in English or when no translation exists.
pub fn t(s: &str) -> &str {
    // Nothing to do for English, for text without letters, or for text that is already
    // Russian (widgets translate their argument, callers may have done so too).
    if get() == Lang::En
        || s.is_empty()
        || !s.chars().any(char::is_alphabetic)
        || s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
    {
        return s;
    }
    match table().get(s) {
        Some(ru) => ru,
        None => {
            report_missing(s);
            s
        }
    }
}

/// Translate a template and fill its `{}` placeholders in order.
pub fn tf(template: &str, args: &[&dyn Display]) -> String {
    let template = t(template);
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(arg) = args.next() {
            out.push_str(&arg.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// `tr!("Move {} to Trash", n)` — translated, formatted.
#[macro_export]
macro_rules! tr {
    ($t:literal) => {
        $crate::i18n::t($t)
    };
    ($t:literal, $($a:expr),+ $(,)?) => {
        $crate::i18n::tf($t, &[$(&$a as &dyn ::std::fmt::Display),+])
    };
}

/// Sizes in the configured units (IEC or SI), with the unit names and decimal
/// separator of the interface language. Every size the interface shows goes
/// through here.
pub fn bytes(n: u64) -> String {
    bytes_in(get(), units(), n)
}

/// Decimal (SI) sizes: `1.50 kB`, `300 B`. Same shape as the IEC formatter.
pub fn format_bytes_si(n: u64) -> String {
    const UNITS: [&str; 5] = ["kB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

fn bytes_in(lang: Lang, units: SizeUnits, n: u64) -> String {
    let s = match units {
        SizeUnits::Iec => space_core::format_bytes(n),
        SizeUnits::Si => format_bytes_si(n),
    };
    if lang == Lang::En {
        return s;
    }
    let s = s.replace('.', ",");
    for (en, ru) in [
        (" TiB", " ТиБ"),
        (" GiB", " ГиБ"),
        (" MiB", " МиБ"),
        (" KiB", " КиБ"),
        (" PB", " ПБ"),
        (" TB", " ТБ"),
        (" GB", " ГБ"),
        (" MB", " МБ"),
        (" kB", " кБ"),
    ] {
        if let Some(num) = s.strip_suffix(en) {
            return format!("{num}{ru}");
        }
    }
    match s.strip_suffix(" B") {
        Some(num) => format!("{num} Б"),
        None => s,
    }
}

/// "1 item" / "3 items" / «1 элемент» / «3 элемента» / «5 элементов».
pub fn count(n: usize, noun: &str) -> String {
    count_in(get(), n, noun)
}

fn count_in(lang: Lang, n: usize, noun: &str) -> String {
    if lang == Lang::En {
        return if n == 1 {
            format!("1 {noun}")
        } else if let Some(stem) = noun.strip_suffix('y') {
            format!("{n} {stem}ies")
        } else {
            format!("{n} {noun}s")
        };
    }
    let (one, few, many) = match noun {
        "item" => ("элемент", "элемента", "элементов"),
        "file" => ("файл", "файла", "файлов"),
        "folder" => ("папка", "папки", "папок"),
        "snapshot" => ("снимок", "снимка", "снимков"),
        "rule" => ("правило", "правила", "правил"),
        "category" => ("категория", "категории", "категорий"),
        "result" => ("результат", "результата", "результатов"),
        _ => (noun, noun, noun),
    };
    let word = match (n % 10, n % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    };
    format!("{n} {word}")
}

/// "3 h ago", "yesterday" and friends.
pub fn age(secs: u64) -> String {
    age_in(get(), secs)
}

fn age_in(lang: Lang, secs: u64) -> String {
    let ru = lang == Lang::Ru;
    let tr = |s: &'static str| {
        if ru {
            table().get(s).copied().unwrap_or(s)
        } else {
            s
        }
    };
    match secs {
        0..=59 => tr("just now").to_string(),
        60..=3599 => {
            let m = secs / 60;
            if ru {
                format!("{m} мин назад")
            } else {
                format!("{m} min ago")
            }
        }
        3600..=86_399 => {
            let h = secs / 3600;
            if ru {
                format!("{h} ч назад")
            } else {
                format!("{h} h ago")
            }
        }
        86_400..=172_799 => tr("yesterday").to_string(),
        _ => {
            let d = secs / 86_400;
            if ru {
                let word = match (d % 10, d % 100) {
                    (_, 11..=14) => "дней",
                    (1, _) => "день",
                    (2..=4, _) => "дня",
                    _ => "дней",
                };
                format!("{d} {word} назад")
            } else {
                format!("{d} days ago")
            }
        }
    }
}

include!("i18n_ru.rs");

/// Does the Russian table translate `s`? For tests of runtime-built labels.
#[cfg(test)]
pub(crate) fn has_ru(s: &str) -> bool {
    RU.iter().any(|(en, _)| *en == s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> usize {
        s.matches("{}").count()
    }

    #[test]
    fn table_keys_are_unique_and_placeholders_match() {
        let mut seen = std::collections::HashSet::new();
        for (en, ru) in RU {
            assert!(seen.insert(*en), "duplicate key {en:?}");
            assert_eq!(
                placeholders(en),
                placeholders(ru),
                "placeholder count differs: {en:?} → {ru:?}"
            );
            assert!(!ru.is_empty(), "empty translation for {en:?}");
        }
    }

    #[test]
    fn russian_plurals_follow_the_rules() {
        let forms = |n| count_in(Lang::Ru, n, "item");
        assert_eq!(forms(1), "1 элемент");
        assert_eq!(forms(2), "2 элемента");
        assert_eq!(forms(5), "5 элементов");
        assert_eq!(forms(11), "11 элементов");
        assert_eq!(forms(14), "14 элементов");
        assert_eq!(forms(21), "21 элемент");
        assert_eq!(forms(22), "22 элемента");
        assert_eq!(forms(111), "111 элементов");
        assert_eq!(count_in(Lang::En, 1, "item"), "1 item");
        assert_eq!(count_in(Lang::En, 3, "item"), "3 items");
        assert_eq!(count_in(Lang::En, 2, "rule"), "2 rules");
        assert_eq!(count_in(Lang::En, 1, "category"), "1 category");
        assert_eq!(count_in(Lang::En, 2, "category"), "2 categories");
        assert_eq!(count_in(Lang::Ru, 1, "rule"), "1 правило");
        assert_eq!(count_in(Lang::Ru, 3, "rule"), "3 правила");
        assert_eq!(count_in(Lang::Ru, 7, "rule"), "7 правил");
        assert_eq!(count_in(Lang::Ru, 1, "category"), "1 категория");
        assert_eq!(count_in(Lang::Ru, 2, "category"), "2 категории");
        assert_eq!(count_in(Lang::Ru, 12, "category"), "12 категорий");
    }

    #[test]
    fn locale_tags_pick_the_language() {
        assert_eq!(lang_of_locale("ru-RU"), Some(Lang::Ru));
        assert_eq!(lang_of_locale("ru_RU.UTF-8"), Some(Lang::Ru));
        assert_eq!(lang_of_locale("RU"), Some(Lang::Ru));
        assert_eq!(lang_of_locale("en-US"), Some(Lang::En));
        assert_eq!(lang_of_locale("de_DE"), Some(Lang::En));
        assert_eq!(lang_of_locale("C"), None);
        assert_eq!(lang_of_locale(""), None);
    }

    #[test]
    fn sizes_and_ages_are_localised() {
        assert_eq!(bytes_in(Lang::En, SizeUnits::Iec, 1536), "1.50 KiB");
        assert_eq!(bytes_in(Lang::Ru, SizeUnits::Iec, 1536), "1,50 КиБ");
        assert_eq!(bytes_in(Lang::Ru, SizeUnits::Iec, 300), "300 Б");
        assert_eq!(bytes_in(Lang::En, SizeUnits::Si, 1536), "1.54 kB");
        assert_eq!(bytes_in(Lang::En, SizeUnits::Si, 2_500_000_000), "2.50 GB");
        assert_eq!(bytes_in(Lang::Ru, SizeUnits::Si, 1536), "1,54 кБ");
        assert_eq!(bytes_in(Lang::Ru, SizeUnits::Si, 999), "999 Б");
        assert_eq!(format_bytes_si(0), "0 B");
        assert_eq!(format_bytes_si(1000), "1.00 kB");
        assert_eq!(age_in(Lang::En, 7200), "2 h ago");
        assert_eq!(age_in(Lang::Ru, 7200), "2 ч назад");
        assert_eq!(age_in(Lang::Ru, 3 * 86_400), "3 дня назад");
        assert_eq!(age_in(Lang::Ru, 5 * 86_400), "5 дней назад");
        assert_eq!(age_in(Lang::Ru, 90_000), "вчера");
    }

    #[test]
    fn templates_fill_placeholders_in_order() {
        // `tf` goes through `t`, which is language-global; check the filler directly.
        let fill = |template: &str, args: &[&dyn Display]| {
            let mut out = String::new();
            let mut args = args.iter();
            let mut rest = template;
            while let Some(i) = rest.find("{}") {
                out.push_str(&rest[..i]);
                if let Some(a) = args.next() {
                    out.push_str(&a.to_string());
                }
                rest = &rest[i + 2..];
            }
            out.push_str(rest);
            out
        };
        assert_eq!(fill("{} of {}", &[&3, &9]), "3 of 9");
    }

    /// Every source file that draws interface text. `i18n_ru.rs` is the table
    /// itself; `volumes.rs` holds device names, not interface strings.
    const SOURCES: &[(&str, &str)] = &[
        ("app/ui/shell.rs", include_str!("app/ui/shell.rs")),
        ("app/ui/scan.rs", include_str!("app/ui/scan.rs")),
        ("app/ui/usage.rs", include_str!("app/ui/usage.rs")),
        ("app/ui/layouts.rs", include_str!("app/ui/layouts.rs")),
        ("app/ui/overview.rs", include_str!("app/ui/overview.rs")),
        ("app/ui/toast.rs", include_str!("app/ui/toast.rs")),
        ("app/ui/cleanup.rs", include_str!("app/ui/cleanup.rs")),
        ("app/ui/review_page.rs", include_str!("app/ui/review_page.rs")),
        ("app/ui/dialogs.rs", include_str!("app/ui/dialogs.rs")),
        ("app.rs", include_str!("app.rs")),
        ("export.rs", include_str!("export.rs")),
        ("junk_panel.rs", include_str!("junk_panel.rs")),
        ("rules_panel.rs", include_str!("rules_panel.rs")),
        ("widgets.rs", include_str!("widgets.rs")),
        ("views/mod.rs", include_str!("views/mod.rs")),
        ("views/treemap.rs", include_str!("views/treemap.rs")),
        ("views/sunburst.rs", include_str!("views/sunburst.rs")),
        ("views/cascade.rs", include_str!("views/cascade.rs")),
        ("views/radial.rs", include_str!("views/radial.rs")),
        ("tm_clean.rs", include_str!("tm_clean.rs")),
        ("theme.rs", include_str!("theme.rs")),
        ("icons.rs", include_str!("icons.rs")),
        ("i18n.rs", include_str!("i18n.rs")),
        ("settings.rs", include_str!("settings.rs")),
    ];

    /// The source without its own test module.
    fn production(src: &str) -> &str {
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    /// Is `at` the start of a call named exactly `call` (not `format!(` for `t(`)?
    fn is_call_start(src: &str, at: usize) -> bool {
        !src[..at]
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '!')
    }

    /// The string literal that starts at `from` after optional whitespace.
    fn literal_after(src: &str, from: usize) -> Option<(usize, &str)> {
        let rest = &src[from..];
        let trimmed = rest.trim_start();
        let open = from + (rest.len() - trimmed.len());
        if !trimmed.starts_with('"') {
            return None;
        }
        let end = src[open + 1..].find('"')?;
        Some((open + 1 + end + 1, &src[open + 1..open + 1 + end]))
    }

    /// Every string literal inside the parentheses of the call at `open`.
    fn literals_in_parens(src: &str, open: usize) -> Vec<&str> {
        let mut depth = 0i32;
        let mut out = Vec::new();
        let mut i = open;
        let bytes = src.as_bytes();
        while i < bytes.len() {
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth <= 0 {
                        break;
                    }
                }
                b'"' => {
                    if let Some(end) = src[i + 1..].find('"') {
                        out.push(&src[i + 1..i + 1 + end]);
                        i += end + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    fn has_words(s: &str) -> bool {
        s.chars().any(char::is_alphabetic)
    }

    /// Every literal handed to a translating call in the drawing code must have
    /// an entry, including calls split over several lines.
    #[test]
    fn every_interface_string_in_the_source_is_translated() {
        // The first literal is the template; later ones are arguments.
        let first_literal = [
            "t(", "tr!(", "primary(", "secondary(", "ghost(", "caption(", "strong(", "title(",
            "section_label(", "danger(", "label: ",
        ];
        // Every literal argument is interface text.
        let every_literal = ["empty_state(", "badge(", "segmented(", "chip(", "stat(", "icons::button("];
        let known: std::collections::HashSet<&str> = RU.iter().map(|(en, _)| *en).collect();
        let mut missing = Vec::new();
        for (name, src) in SOURCES {
            let src = production(src);
            for call in first_literal {
                let mut from = 0;
                while let Some(i) = src[from..].find(call) {
                    let at = from + i;
                    from = at + call.len();
                    if !is_call_start(src, at) {
                        continue;
                    }
                    if let Some((_, lit)) = literal_after(src, from) {
                        if has_words(lit) && !known.contains(lit) {
                            missing.push(format!("{name}: {lit:?}"));
                        }
                    }
                }
            }
            for call in every_literal {
                let mut from = 0;
                while let Some(i) = src[from..].find(call) {
                    let at = from + i;
                    from = at + call.len();
                    if !is_call_start(src, at) {
                        continue;
                    }
                    for lit in literals_in_parens(src, from - 1) {
                        if has_words(&without_directives(lit)) && !known.contains(lit) {
                            missing.push(format!("{name}: {lit:?}"));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "no Russian translation for: {missing:#?}"
        );
    }

    /// `lit` with its `{…}` format directives removed.
    fn without_directives(lit: &str) -> String {
        let mut out = String::new();
        let mut depth = 0;
        let mut escaped = false;
        for c in lit.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '{' => depth += 1,
                '}' => depth = (depth - 1).max(0),
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// Text drawn with the painter or shown as a hover hint, checkbox label or
    /// rich text must be a translation key reached through `t()`/`tr!()`; a
    /// raw English literal there would never be translated.
    #[test]
    fn no_interface_literal_bypasses_translation() {
        let calls = [
            "on_hover_text(",
            ".text(",
            "RichText::new(",
            ".checkbox(",
            ".label(",
            ".button(",
            "selectable_label(",
            "hint_text(",
            "errors.push(",
        ];
        let known: std::collections::HashSet<&str> = RU.iter().map(|(en, _)| *en).collect();
        // Proper nouns and the brand are the same in every language; the
        // nouns are `count()` arguments, pluralised by the language tables.
        let allowed = [
            "Time Machine", "Space", "item", "file", "folder", "snapshot", "rule", "category", "result",
        ];
        let mut raw = Vec::new();
        for (name, src) in SOURCES {
            let src = production(src);
            for call in calls {
                let mut from = 0;
                while let Some(i) = src[from..].find(call) {
                    let at = from + i;
                    from = at + call.len();
                    for lit in literals_in_parens(src, from - 1) {
                        if has_words(&without_directives(lit))
                            && !known.contains(lit)
                            && !allowed.contains(&lit)
                        {
                            raw.push(format!("{name}: {lit:?}"));
                        }
                    }
                }
            }
        }
        raw.sort();
        raw.dedup();
        assert!(raw.is_empty(), "interface text that bypasses t(): {raw:#?}");
    }

    /// Every Russian key must still be used by some source file, so stale
    /// entries do not pile up as the interface changes.
    #[test]
    fn no_orphan_ru_keys() {
        // Keys built at runtime from data outside this crate: rule-pack
        // category names, risk names and tag labels.
        let dynamic = [
            "Caches", "Logs", "Temp files", "Leftovers", "Developer", "Package caches",
            "Browser caches", "App caches", "Media caches", "ML caches", "ML models",
            "VM caches", "VM disks", "Game caches", "Game downloads", "Shader caches",
            "Wine caches", "Databases", "App data", "System", "Media libraries",
            "Sample libraries", "Privacy",
        ];
        let sources: String = SOURCES.iter().map(|(_, src)| *src).collect::<Vec<_>>().join("\n");
        let mut orphans = Vec::new();
        for (en, _) in RU {
            if dynamic.contains(en) {
                continue;
            }
            if !sources.contains(&format!("{en:?}")) {
                orphans.push(*en);
            }
        }
        assert!(orphans.is_empty(), "Russian keys no source uses: {orphans:#?}");
    }
}
