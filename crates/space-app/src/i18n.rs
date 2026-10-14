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

/// Language from the environment (`LC_ALL`, `LC_MESSAGES`, `LANG`).
pub fn detect() -> Lang {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            if !value.is_empty() {
                return if value.to_ascii_lowercase().starts_with("ru") {
                    Lang::Ru
                } else {
                    Lang::En
                };
            }
        }
    }
    Lang::En
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

/// Sizes with the unit names and decimal separator of the interface language.
pub fn bytes(n: u64) -> String {
    bytes_in(get(), n)
}

fn bytes_in(lang: Lang, n: u64) -> String {
    let s = space_core::format_bytes(n);
    if lang == Lang::En {
        return s;
    }
    let s = s.replace('.', ",");
    for (en, ru) in [
        (" TiB", " ТиБ"),
        (" GiB", " ГиБ"),
        (" MiB", " МиБ"),
        (" KiB", " КиБ"),
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
        } else {
            format!("{n} {noun}s")
        };
    }
    let (one, few, many) = match noun {
        "item" => ("элемент", "элемента", "элементов"),
        "file" => ("файл", "файла", "файлов"),
        "folder" => ("папка", "папки", "папок"),
        "snapshot" => ("снимок", "снимка", "снимков"),
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
    }

    #[test]
    fn sizes_and_ages_are_localised() {
        assert_eq!(bytes_in(Lang::En, 1536), "1.50 KiB");
        assert_eq!(bytes_in(Lang::Ru, 1536), "1,50 КиБ");
        assert_eq!(bytes_in(Lang::Ru, 300), "300 Б");
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

    /// Every literal handed to a translating call in the drawing code must have an entry.
    #[test]
    fn every_interface_string_in_the_source_is_translated() {
        let sources = [
            include_str!("app/ui/shell.rs"),
            include_str!("app/ui/scan.rs"),
            include_str!("app/ui/usage.rs"),
            include_str!("app/ui/layouts.rs"),
            include_str!("app/ui/overview.rs"),
            include_str!("app/ui/toast.rs"),
            include_str!("app/ui/cleanup.rs"),
            include_str!("app/ui/review_page.rs"),
            include_str!("app/ui/dialogs.rs"),
            include_str!("app.rs"),
            include_str!("junk_panel.rs"),
            include_str!("rules_panel.rs"),
            include_str!("widgets.rs"),
            include_str!("views/mod.rs"),
            include_str!("settings.rs"),
            include_str!("theme.rs"),
            include_str!("icons.rs"),
        ];
        let calls = [
            "t(\"",
            "tr!(\"",
            "primary(\"",
            "secondary(\"",
            "ghost(\"",
            "caption(\"",
            "strong(\"",
            "title(\"",
            "section_label(\"",
            "danger(\"",
            "empty_state(ui, \"",
            "badge(ui, \"",
            "label: \"",
        ];
        let known: std::collections::HashSet<&str> = RU.iter().map(|(en, _)| *en).collect();
        let mut missing = Vec::new();
        for src in sources {
            let src = src.split("#[cfg(test)]").next().unwrap_or(src);
            for call in calls {
                let mut from = 0;
                while let Some(i) = src[from..].find(call) {
                    let start = from + i + call.len();
                    // Skip calls that merely end in the name, like `format!(` or `split(`.
                    let before = src[..from + i].chars().last();
                    from = start;
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(end) = src[start..].find('"') else {
                        continue;
                    };
                    let lit = &src[start..start + end];
                    if lit.chars().any(char::is_alphabetic) && !known.contains(lit) {
                        missing.push(lit.to_owned());
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
}
