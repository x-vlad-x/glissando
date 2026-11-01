//! UI translations (English source strings; Russian and German).

use std::collections::HashMap;
use std::sync::OnceLock;

const RU: &[(&str, &str)] = &[
    ("Keyboard settings", "Настройки клавиатуры"),
    ("Languages", "Языки"),
    ("Layouts and multilingual typing", "Раскладки и многоязычный ввод"),
    ("Keyboard layouts", "Раскладки"),
    (
        "Enabled layouts are switched with the globe key, in the order shown.",
        "Включённые раскладки переключаются клавишей с глобусом в указанном порядке.",
    ),
    ("Move up", "Выше"),
    ("Multilingual typing", "Многоязычный ввод"),
    (
        "Suggest and correct in all enabled languages that share an alphabet",
        "Подсказки и исправления во всех включённых языках с общим алфавитом",
    ),
    ("Preferences", "Параметры"),
    ("Keys, layout, height, vibration", "Клавиши, вид, высота, вибрация"),
    ("Keys", "Клавиши"),
    ("Number row", "Ряд цифр"),
    ("Always show digits above the letters", "Всегда показывать цифры над буквами"),
    ("Terminal keys", "Клавиши терминала"),
    (
        "Esc, Tab, Ctrl, Alt and arrows above the letters in terminals",
        "Esc, Tab, Ctrl, Alt и стрелки над буквами в терминалах",
    ),
    ("Touch & hold for digits and symbols", "Цифры и символы долгим нажатием"),
    ("Show hints on letter keys", "Показывать подсказки на клавишах"),
    ("Key pop-up", "Увеличение нажатой клавиши"),
    ("Show the pressed key above the finger", "Показывать нажатую клавишу над пальцем"),
    ("Touch & hold duration (ms)", "Длительность долгого нажатия (мс)"),
    ("Layout", "Вид"),
    ("One-handed mode", "Режим одной руки"),
    ("Off", "Выкл."),
    ("Left", "Слева"),
    ("Right", "Справа"),
    ("Keyboard height (portrait)", "Высота клавиатуры (портрет)"),
    ("Keyboard height (landscape)", "Высота клавиатуры (альбом)"),
    ("Key tap", "Нажатие клавиш"),
    ("Vibrate on keypress", "Вибрация при нажатии"),
    ("Vibration strength (%)", "Сила вибрации (%)"),
    ("Vibration duration (ms)", "Длительность вибрации (мс)"),
    ("Theme", "Тема"),
    ("Light, dark, font", "Светлая, тёмная, шрифт"),
    ("Appearance", "Внешний вид"),
    ("Follow system", "Как в системе"),
    ("Light", "Светлая"),
    ("Dark", "Тёмная"),
    ("Key borders", "Границы клавиш"),
    ("Higher contrast between keys", "Более контрастные клавиши"),
    ("Font size", "Размер шрифта"),
    ("Font family", "Шрифт"),
    ("Glide typing", "Ввод свайпом"),
    ("Gestures and trail", "Жесты и след"),
    ("Type words by sliding from letter to letter", "Вводите слова, проводя пальцем от буквы к букве"),
    ("Glide trail", "След свайпа"),
    ("Show how the finger moves", "Показывать движение пальца"),
    ("Glide delete", "Удаление свайпом"),
    ("Delete words by sliding left from the delete key", "Удалять слова движением влево от клавиши удаления"),
    ("Glide cursor control", "Управление курсором"),
    ("Slide on the space bar to move the cursor", "Двигайте курсор, проводя по пробелу"),
    ("Trackpad on long-press space", "Тачпад при долгом нажатии пробела"),
    ("Text correction", "Исправление текста"),
    ("Suggestions, auto-correction, capitalization", "Подсказки, автоисправление, заглавные"),
    ("Corrections & suggestions", "Исправления и подсказки"),
    ("Suggestion strip", "Панель подсказок"),
    ("Show word suggestions while typing", "Показывать варианты слов при наборе"),
    ("Next-word suggestions", "Подсказка следующего слова"),
    ("Auto-correction", "Автоисправление"),
    ("Auto-capitalization", "Заглавная в начале предложения"),
    ("Auto-space after suggestions", "Пробел после подсказки"),
    ("Double-space period", "Точка двойным пробелом"),
    ("Underline the word being typed", "Подчёркивать набираемое слово"),
    ("Turn off for apps with broken preedit support", "Отключите для приложений с некорректной поддержкой preedit"),
    ("Clipboard", "Буфер обмена"),
    ("History and retention", "История и срок хранения"),
    ("Clipboard history", "История буфера обмена"),
    ("Keep copied text in memory to paste it later", "Хранить скопированный текст в памяти для вставки"),
    ("Keep history for (hours)", "Хранить историю (часов)"),
    (
        "Text copied from password managers is never recorded. History is kept in memory only; pinned items are stored on this device.",
        "Текст из менеджеров паролей не сохраняется. История хранится только в памяти; закреплённые элементы — на этом устройстве.",
    ),
    ("Privacy & dictionary", "Конфиденциальность и словарь"),
    ("Learned words, delete data", "Выученные слова, удаление данных"),
    ("Privacy", "Конфиденциальность"),
    (
        "glissando works entirely on this device. It has no network access, no telemetry and no accounts. Nothing you type leaves the phone.",
        "glissando работает полностью на устройстве: без доступа к сети, телеметрии и учётных записей. Набранный текст не покидает телефон.",
    ),
    ("Personalization", "Персонализация"),
    (
        "Learn new words and your word order on this device (never in password fields)",
        "Запоминать новые слова и порядок слов на устройстве (никогда в полях паролей)",
    ),
    ("Remember recent emoji", "Запоминать недавние эмодзи"),
    ("Delete learned words and data", "Удалить выученные слова и данные"),
    (
        "Personal dictionary, learned words, recent emoji and pinned clipboard items",
        "Личный словарь, выученные слова, недавние эмодзи и закреплённые элементы буфера",
    ),
    ("Delete", "Удалить"),
    ("Delete all personal data?", "Удалить все личные данные?"),
    ("This cannot be undone.", "Это действие нельзя отменить."),
    ("Cancel", "Отмена"),
    ("Personal dictionary", "Личный словарь"),
    ("Add a word", "Добавить слово"),
    ("added by you", "добавлено вами"),
    ("learned", "выучено"),
    ("Forget this word", "Забыть слово"),
    ("About", "О программе"),
    ("Version and licences", "Версия и лицензии"),
    ("Version", "Версия"),
    (
        "Free software under the GNU GPL v3 or later. Word lists: Android Open Source Project (Apache-2.0); word pair statistics: Leipzig Corpora Collection (CC BY 4.0); emoji names: Unicode CLDR (Unicode License v3).",
        "Свободное ПО по лицензии GNU GPL v3 или новее. Списки слов: Android Open Source Project (Apache-2.0); статистика пар слов: Leipzig Corpora Collection (CC BY 4.0); названия эмодзи: Unicode CLDR (Unicode License v3).",
    ),
    ("Configuration file", "Файл настроек"),
    ("Voice typing", "Голосовой ввод"),
    (
        "Local speech recognition with whisper.cpp; audio never leaves the device",
        "Локальное распознавание речи whisper.cpp; звук не покидает устройство",
    ),
    ("Personal data", "Личные данные"),
    ("Apps", "Приложения"),
    ("Where the keyboard does not open by itself", "Где клавиатура не открывается сама"),
    ("Don't open automatically in", "Не открывать автоматически в"),
    (
        "In these apps the keyboard does not appear when a text field gets focus, for example in a calculator with its own keypad. It can still be opened from the bottom bar.",
        "В этих приложениях клавиатура не появляется при выборе текстового поля — например, в калькуляторе со своими кнопками. Её по-прежнему можно открыть с нижней панели.",
    ),
    ("Add app", "Добавить приложение"),
    ("Add by app ID", "Добавить по ID приложения"),
    ("Import from the Phosh keyboard (Stevia)", "Импорт из клавиатуры Phosh (Stevia)"),
    ("Import", "Импорт"),
    ("Remove", "Удалить"),
];

const DE: &[(&str, &str)] = &[
    ("Keyboard settings", "Tastatureinstellungen"),
    ("Languages", "Sprachen"),
    ("Layouts and multilingual typing", "Layouts und mehrsprachige Eingabe"),
    ("Keyboard layouts", "Tastaturlayouts"),
    ("Multilingual typing", "Mehrsprachige Eingabe"),
    ("Preferences", "Einstellungen"),
    ("Keys", "Tasten"),
    ("Number row", "Zahlenreihe"),
    ("Terminal keys", "Terminaltasten"),
    (
        "Esc, Tab, Ctrl, Alt and arrows above the letters in terminals",
        "Esc, Tab, Strg, Alt und Pfeile über den Buchstaben in Terminals",
    ),
    ("Key pop-up", "Tasten-Pop-up"),
    ("Layout", "Layout"),
    ("One-handed mode", "Einhandmodus"),
    ("Off", "Aus"),
    ("Left", "Links"),
    ("Right", "Rechts"),
    ("Key tap", "Tastendruck"),
    ("Vibrate on keypress", "Vibration bei Tastendruck"),
    ("Theme", "Design"),
    ("Appearance", "Darstellung"),
    ("Follow system", "Wie System"),
    ("Light", "Hell"),
    ("Dark", "Dunkel"),
    ("Glide typing", "Gleitende Eingabe"),
    ("Glide delete", "Gleitendes Löschen"),
    ("Glide cursor control", "Cursorsteuerung"),
    ("Text correction", "Textkorrektur"),
    ("Corrections & suggestions", "Korrekturen und Vorschläge"),
    ("Suggestion strip", "Vorschlagsleiste"),
    ("Next-word suggestions", "Vorschläge für nächstes Wort"),
    ("Auto-correction", "Autokorrektur"),
    ("Auto-capitalization", "Automatische Großschreibung"),
    ("Double-space period", "Punkt durch doppeltes Leerzeichen"),
    ("Clipboard", "Zwischenablage"),
    ("Clipboard history", "Verlauf der Zwischenablage"),
    ("Privacy & dictionary", "Datenschutz und Wörterbuch"),
    ("Privacy", "Datenschutz"),
    ("Personalization", "Personalisierung"),
    ("Delete learned words and data", "Gelernte Wörter und Daten löschen"),
    ("Delete", "Löschen"),
    ("Cancel", "Abbrechen"),
    ("Personal dictionary", "Persönliches Wörterbuch"),
    ("Add a word", "Wort hinzufügen"),
    ("About", "Info"),
    ("Version", "Version"),
    ("Voice typing", "Spracheingabe"),
    ("Apps", "Apps"),
    ("Where the keyboard does not open by itself", "Wo sich die Tastatur nicht selbst öffnet"),
    ("Don't open automatically in", "Nicht automatisch öffnen in"),
    (
        "In these apps the keyboard does not appear when a text field gets focus, for example in a calculator with its own keypad. It can still be opened from the bottom bar.",
        "In diesen Apps erscheint die Tastatur nicht, wenn ein Textfeld den Fokus erhält, etwa in einem Taschenrechner mit eigenen Tasten. Über die untere Leiste lässt sie sich weiterhin öffnen.",
    ),
    ("Add app", "App hinzufügen"),
    ("Add by app ID", "Über App-ID hinzufügen"),
    ("Import from the Phosh keyboard (Stevia)", "Aus der Phosh-Tastatur (Stevia) übernehmen"),
    ("Import", "Übernehmen"),
    ("Remove", "Entfernen"),
];

fn table() -> Option<&'static HashMap<&'static str, &'static str>> {
    static T: OnceLock<Option<HashMap<&'static str, &'static str>>> = OnceLock::new();
    T.get_or_init(|| {
        let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        let src = if lang.starts_with("ru") {
            RU
        } else if lang.starts_with("de") {
            DE
        } else {
            return None;
        };
        Some(src.iter().copied().collect())
    })
    .as_ref()
}

pub fn tr(s: &'static str) -> &'static str {
    table().and_then(|t| t.get(s).copied()).unwrap_or(s)
}
