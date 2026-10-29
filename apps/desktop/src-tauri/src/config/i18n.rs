use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
    Ko,
    Ru,
}

static CURRENT_LANG: AtomicU8 = AtomicU8::new(0); // 0 = zh, 1 = en, 2 = ko, 3 = ru

pub fn set_language(lang: Lang) {
    CURRENT_LANG.store(
        match lang {
            Lang::Zh => 0,
            Lang::En => 1,
            Lang::Ko => 2,
            Lang::Ru => 3,
        },
        Ordering::SeqCst,
    );
}

pub fn lang_from_setting(language: &str) -> Lang {
    match language {
        "en" | "en-US" => Lang::En,
        "ko" | "ko-KR" => Lang::Ko,
        "ru" | "ru-RU" => Lang::Ru,
        _ => Lang::Zh,
    }
}

fn lang() -> Lang {
    match CURRENT_LANG.load(Ordering::SeqCst) {
        1 => Lang::En,
        2 => Lang::Ko,
        3 => Lang::Ru,
        _ => Lang::Zh,
    }
}

fn pick(key: &str, lang: Lang) -> String {
    let (zh, en, ko, ru): (&str, &str, &str, &str) = match key {
        "menu.open_panel" => ("打开面板", "Open Panel", "패널 열기", "Открыть панель"),
        "menu.quit" => ("退出", "Quit", "종료", "Выход"),
        _ => (key, key, key, key),
    };
    match lang {
        Lang::Zh => zh.to_string(),
        Lang::En => en.to_string(),
        Lang::Ko => ko.to_string(),
        Lang::Ru => ru.to_string(),
    }
}

pub fn t(key: &str) -> String {
    pick(key, lang())
}

