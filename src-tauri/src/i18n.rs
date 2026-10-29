//! The texts Kova shows outside its window: the tray menu and notifications raised
//! from here. The window translates itself (ui/src/lib/i18n.svelte.ts).

use kova_engine::store::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
    Fa,
    Zh,
}

/// The system's display language as a BCP 47 tag, such as "ru-RU".
pub fn system_language() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".into())
}

/// The language to show for the `language` setting ("system" or a language code).
pub fn lang(s: &Settings) -> Lang {
    let tag = match s.language.as_str() {
        "en" | "ru" | "fa" | "zh" => s.language.clone(),
        _ => system_language(),
    };
    let tag = tag.to_ascii_lowercase();
    if tag.starts_with("ru") {
        Lang::Ru
    } else if tag.starts_with("fa") {
        Lang::Fa
    } else if tag.starts_with("zh") {
        Lang::Zh
    } else {
        Lang::En
    }
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
    ("Server", "Сервер"),
    ("Profile", "Профиль"),
    ("Quit Kova", "Выйти из Kova"),
    ("Kova: connected ({})", "Kova: подключено ({})"),
    ("Kova: disconnected", "Kova: отключено"),
    ("Downloading {}…", "Загрузка {}…"),
    ("{} {} installed", "{} {} установлен"),
    ("Could not install {}: {}", "Не удалось установить {}: {}"),
    ("Could not connect: {}", "Не удалось подключиться: {}"),
    ("{} stopped answering; switched to {}", "{} перестал отвечать; переключено на {}"),
    ("Another program changed the system proxy, so apps no longer go through Kova. Turn System proxy off and on to take it back", "Другая программа изменила системный прокси, и приложения больше не идут через Kova. Выключите и снова включите «Системный прокси», чтобы вернуть его"),
    ("{} has expired", "Подписка {} закончилась"),
    ("{} ends in {} days", "Подписка {} закончится через {} дн."),
    ("{} has used {}% of its traffic", "Подписка {} израсходовала {}% трафика"),
];

const FA: &[(&str, &str)] = &[
    ("Connect", "اتصال"),
    ("Disconnect", "قطع اتصال"),
    ("Routing mode", "حالت مسیریابی"),
    ("Rule", "قوانین"),
    ("Global", "سراسری"),
    ("Direct", "مستقیم"),
    ("System proxy", "پراکسی سیستم"),
    ("TUN mode", "حالت TUN"),
    ("Open Kova", "باز کردن Kova"),
    ("Server", "سرور"),
    ("Profile", "پروفایل"),
    ("Quit Kova", "خروج از Kova"),
    ("Kova: connected ({})", "Kova: متصل ({})"),
    ("Kova: disconnected", "Kova: قطع"),
    ("Downloading {}…", "در حال دانلود {}…"),
    ("{} {} installed", "{} {} نصب شد"),
    ("Could not install {}: {}", "نصب {} ممکن نشد: {}"),
    ("Could not connect: {}", "اتصال ممکن نشد: {}"),
    ("{} stopped answering; switched to {}", "{} پاسخ نمی‌دهد؛ به {} تغییر کرد"),
    ("Another program changed the system proxy, so apps no longer go through Kova. Turn System proxy off and on to take it back", "برنامهٔ دیگری پراکسی سیستم را تغییر داد و برنامه‌ها دیگر از Kova عبور نمی‌کنند. برای بازگرداندن، «پراکسی سیستم» را خاموش و دوباره روشن کنید"),
    ("{} has expired", "اشتراک {} منقضی شده است"),
    ("{} ends in {} days", "اشتراک {} تا {} روز دیگر تمام می‌شود"),
    ("{} has used {}% of its traffic", "اشتراک {} {}% از ترافیکش را مصرف کرده است"),
];

const ZH: &[(&str, &str)] = &[
    ("Connect", "连接"),
    ("Disconnect", "断开连接"),
    ("Routing mode", "路由模式"),
    ("Rule", "规则"),
    ("Global", "全局"),
    ("Direct", "直连"),
    ("System proxy", "系统代理"),
    ("TUN mode", "TUN 模式"),
    ("Open Kova", "打开 Kova"),
    ("Server", "服务器"),
    ("Profile", "配置"),
    ("Quit Kova", "退出 Kova"),
    ("Kova: connected ({})", "Kova：已连接（{}）"),
    ("Kova: disconnected", "Kova：已断开"),
    ("Downloading {}…", "正在下载 {}…"),
    ("{} {} installed", "已安装 {} {}"),
    ("Could not install {}: {}", "无法安装 {}：{}"),
    ("Could not connect: {}", "无法连接：{}"),
    ("{} stopped answering; switched to {}", "{} 无响应；已切换到 {}"),
    ("Another program changed the system proxy, so apps no longer go through Kova. Turn System proxy off and on to take it back", "其他程序更改了系统代理，应用不再经过 Kova。关闭再打开“系统代理”即可恢复"),
    ("{} has expired", "{} 已过期"),
    ("{} ends in {} days", "{} 将在 {} 天后到期"),
    ("{} has used {}% of its traffic", "{} 已使用 {}% 的流量"),
];

/// `en` in `lang`, with each `{}` replaced by the next of `args`.
pub fn tr(lang: Lang, en: &'static str, args: &[&str]) -> String {
    let table = match lang {
        Lang::En => return fill(en, args),
        Lang::Ru => RU,
        Lang::Fa => FA,
        Lang::Zh => ZH,
    };
    fill(table.iter().find(|(k, _)| *k == en).map_or(en, |(_, v)| v), args)
}

fn fill(text: &str, args: &[&str]) -> String {
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
        for table in [RU, FA, ZH] {
            assert_eq!(table.len(), RU.len());
            for (en, t) in table {
                assert_eq!(en.matches("{}").count(), t.matches("{}").count(), "{en}");
            }
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
        let mut s = Settings { language: "ru".into(), ..Default::default() };
        assert_eq!(lang(&s), Lang::Ru);
        s.language = "en".into();
        assert_eq!(lang(&s), Lang::En);
        s.language = "fa".into();
        assert_eq!(lang(&s), Lang::Fa);
        s.language = "zh".into();
        assert_eq!(lang(&s), Lang::Zh);
    }
}
