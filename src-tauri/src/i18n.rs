//! The texts Kova shows outside its window: the tray menu and notifications raised
//! from here. The window translates itself (ui/src/lib/i18n.svelte.ts).

use kova_engine::store::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

/// The system's display language as a BCP 47 tag, such as "ru-RU".
pub fn system_language() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".into())
}

/// The language to show for the `language` setting ("system", "en" or "ru").
pub fn lang(s: &Settings) -> Lang {
    let tag = match s.language.as_str() {
        "en" | "ru" => s.language.clone(),
        _ => system_language(),
    };
    if tag.to_ascii_lowercase().starts_with("ru") { Lang::Ru } else { Lang::En }
}

const RU: &[(&str, &str)] = &[
    ("Connect", "Подключить"),
    ("Disconnect", "Отключить"),
    ("Routing mode", "Режим маршрутизации"),
    ("Rule", "По правилам"),
    ("Global", "Глобальный"),
    ("Direct", "Напрямую"),
    ("System proxy", "Системный прокси"),
    ("TUN mode", "Режим TUN"),
    ("Open Kova", "Открыть Kova"),
    ("Quit Kova", "Выйти из Kova"),
    ("Kova: connected ({})", "Kova: подключено ({})"),
    ("Kova: disconnected", "Kova: отключено"),
    ("Downloading {}…", "Загрузка {}…"),
    ("{} {} installed", "{} {} установлен"),
    ("Could not install {}: {}", "Не удалось установить {}: {}"),
    ("Could not connect: {}", "Не удалось подключиться: {}"),
    ("{} stopped answering; switched to {}", "{} перестал отвечать; переключено на {}"),
    ("{} has expired", "Подписка {} закончилась"),
    ("{} ends in {} days", "Подписка {} закончится через {} дн."),
    ("{} has used {}% of its traffic", "Подписка {} израсходовала {}% трафика"),
];

/// `en` in `lang`, with each `{}` replaced by the next of `args`.
pub fn tr(lang: Lang, en: &'static str, args: &[&str]) -> String {
    let text = match lang {
        Lang::En => en,
        Lang::Ru => RU.iter().find(|(k, _)| *k == en).map_or(en, |(_, v)| v),
    };
    let mut parts = text.split("{}");
    let mut out = parts.next().unwrap_or_default().to_string();
    for (i, part) in parts.enumerate() {
        out.push_str(args.get(i).copied().unwrap_or_default());
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_keep_their_placeholders() {
        for (en, ru) in RU {
            assert_eq!(en.matches("{}").count(), ru.matches("{}").count(), "{en}");
        }
    }

    #[test]
    fn fills_arguments_in_order() {
        assert_eq!(tr(Lang::En, "{} {} installed", &["Xray", "v26.1"]), "Xray v26.1 installed");
        assert_eq!(tr(Lang::Ru, "Could not connect: {}", &["timeout"]), "Не удалось подключиться: timeout");
        assert_eq!(tr(Lang::Ru, "Not in the table", &[]), "Not in the table");
    }

    #[test]
    fn explicit_setting_wins_over_the_system() {
        let mut s = Settings::default();
        s.language = "ru".into();
        assert_eq!(lang(&s), Lang::Ru);
        s.language = "en".into();
        assert_eq!(lang(&s), Lang::En);
    }
}
