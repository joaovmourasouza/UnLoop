//! Enums, tabelas de ícones/cores e entidades.
//!
//! Os códigos inteiros e os nomes "legacy" seguem o Quitzilla 3.0.2 para que
//! backups dele possam ser importados sem perda.

use crate::units::MS_PER_DAY;

macro_rules! coded_enum {
    ($name:ident, default = $default:ident, { $($variant:ident = ($code:expr, $legacy:expr, $label:expr)),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn code(self) -> i64 {
                match self { $($name::$variant => $code),+ }
            }

            pub fn legacy(self) -> &'static str {
                match self { $($name::$variant => $legacy),+ }
            }

            pub fn label(self) -> &'static str {
                match self { $($name::$variant => $label),+ }
            }

            pub fn from_code(code: i64) -> Self {
                Self::ALL.iter().copied().find(|v| v.code() == code).unwrap_or($name::$default)
            }

            pub fn from_legacy(name: &str) -> Self {
                Self::ALL.iter().copied().find(|v| v.legacy() == name).unwrap_or($name::$default)
            }

            /// Posição em [`Self::ALL`] (útil para listas de seleção na UI).
            pub fn index(self) -> usize {
                Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
            }

            pub fn from_index(index: usize) -> Self {
                Self::ALL.get(index).copied().unwrap_or($name::$default)
            }
        }

        impl Default for $name {
            fn default() -> Self { $name::$default }
        }
    };
}

coded_enum!(CounterType, default = Abstinence, {
    Abstinence = (300, "ABSTINENCE", "Tempo limpo (conta para frente)"),
    Remaining = (200, "REMAINING", "Tempo até a meta (conta para trás)"),
});

coded_enum!(CounterStyle, default = Full, {
    Full = (400, "FULL", "Completo (compacto)"),
    FullLabeled = (401, "FULL_SHORT", "Completo (por extenso)"),
    DaysHoursMinutes = (411, "DAYS_HOURS_MINUTES", "Dias, horas e minutos"),
    DaysHours = (402, "DAYS_HOURS", "Dias e horas"),
    WeeksDays = (403, "WEEKS_DAYS", "Semanas e dias"),
    MonthsDays = (404, "MONTHS_DAYS", "Meses e dias"),
    YearsMonthsDays = (405, "YEARS_MONTHS_DAYS", "Anos, meses e dias"),
    Minutes = (412, "MINUTES", "Minutos"),
    Hours = (406, "HOURS", "Horas"),
    Days = (407, "DAYS", "Dias"),
    Weeks = (408, "WEEKS", "Semanas"),
    Months = (409, "MONTHS", "Meses"),
});

coded_enum!(MetricType, default = Money, {
    Money = (0, "MONEY", "Dinheiro"),
    Time = (1, "TIME", "Tempo"),
});

// Troféus: conquistados quando QUALQUER período sem recaída (passado ou atual)
// alcança o limiar. Uma recaída não remove troféus.
coded_enum!(Trophy, default = DecisionMade, {
    DecisionMade = (1, "DECISION_MADE", "Decisão tomada"),
    Day1 = (2, "DAY_1", "1 dia"),
    Day3 = (3, "DAY_3", "3 dias"),
    Day7 = (4, "DAY_7", "1 semana"),
    Day10 = (5, "DAY_10", "10 dias"),
    Day14 = (6, "DAY_14", "2 semanas"),
    Day30 = (7, "DAY_30", "30 dias"),
    Day90 = (8, "DAY_90", "90 dias"),
    Day180 = (9, "DAY_180", "Meio ano"),
    Day365 = (10, "DAY_365", "1 ano"),
    Day1825 = (11, "DAY_1825", "5 anos"),
    Day3650 = (12, "DAY_3650", "10 anos"),
});

// Tipo do hábito: parar (contador desde a última recaída) ou criar (check diário).
coded_enum!(HabitKind, default = Quit, {
    Quit = (0, "QUIT", "Parar um hábito (contador)"),
    Build = (1, "BUILD", "Criar um hábito (check diário)"),
});

// Regras do hábito: o que é permitido (não reinicia) e o que não é (reinicia).
coded_enum!(RuleKind, default = Forbidden, {
    Allowed = (0, "ALLOWED", "Permitido"),
    Forbidden = (1, "FORBIDDEN", "Não permitido"),
});

impl RuleKind {
    /// Uma entrada do diário ligada a esta regra reinicia a sequência?
    pub fn resets_streak(self) -> bool {
        self == RuleKind::Forbidden
    }

    pub fn emoji(self) -> &'static str {
        match self {
            RuleKind::Allowed => "✅",
            RuleKind::Forbidden => "⛔",
        }
    }
}

impl Trophy {
    /// Limiar em ms (DAY_180 = 182,5 dias, como no original).
    pub fn ms(self) -> i64 {
        match self {
            Trophy::DecisionMade => 0,
            Trophy::Day1 => MS_PER_DAY,
            Trophy::Day3 => 3 * MS_PER_DAY,
            Trophy::Day7 => 7 * MS_PER_DAY,
            Trophy::Day10 => 10 * MS_PER_DAY,
            Trophy::Day14 => 14 * MS_PER_DAY,
            Trophy::Day30 => 30 * MS_PER_DAY,
            Trophy::Day90 => 90 * MS_PER_DAY,
            Trophy::Day180 => 15_768_000_000,
            Trophy::Day365 => 365 * MS_PER_DAY,
            Trophy::Day1825 => 1825 * MS_PER_DAY,
            Trophy::Day3650 => 3650 * MS_PER_DAY,
        }
    }
}

// ---------------------------------------------------------------- Ícones

#[derive(Debug)]
pub struct IconInfo {
    pub code: i64,
    pub legacy: &'static str,
    pub emoji: &'static str,
    pub label: &'static str,
}

const fn ic(code: i64, legacy: &'static str, emoji: &'static str, label: &'static str) -> IconInfo {
    IconInfo { code, legacy, emoji, label }
}

pub const ICONS: &[IconInfo] = &[
    ic(2001, "GENERIC", "🚫", "Meu hábito"),
    ic(2002, "SMOKING", "🚬", "Cigarro"),
    ic(2003, "ALCOHOL", "🍺", "Álcool"),
    ic(2004, "DRUGS", "💉", "Drogas"),
    ic(2005, "PILLS", "💊", "Remédios"),
    ic(2006, "CANNABIS", "🌿", "Maconha"),
    ic(2007, "FASTFOOD", "🍔", "Fast food"),
    ic(2008, "SUGAR", "🍬", "Açúcar"),
    ic(2009, "PORN", "🔞", "Pornografia"),
    ic(2010, "GAMES", "🎮", "Videogame"),
    ic(2011, "TELEVISION", "📺", "Televisão"),
    ic(2012, "SHOPPING", "🛍️", "Compras por impulso"),
    ic(2013, "FACEBOOK", "👥", "Facebook"),
    ic(2014, "TWITTER", "🐦", "X / Twitter"),
    ic(2015, "INSTAGRAM", "📸", "Instagram"),
    ic(2016, "GAMBLING", "🎰", "Apostas"),
    ic(2017, "REDDIT", "👽", "Reddit"),
    ic(2018, "YOUTUBE", "▶️", "YouTube"),
    ic(2019, "OVEREATING", "🍽️", "Comer demais"),
    ic(20120, "SODA", "🥤", "Refrigerante"),
    ic(2021, "COFFEE", "☕", "Café"),
    ic(2022, "PROCRASTINATION", "⏳", "Procrastinação"),
    ic(2023, "CURSING", "🤬", "Palavrões"),
    ic(2024, "EATING_MEAT", "🥩", "Carne"),
    ic(2025, "LIE", "🤥", "Mentir"),
    ic(2026, "QUARREL", "😤", "Brigas"),
    ic(2027, "BITING_FINGERNAILS", "💅", "Roer unhas"),
    ic(2028, "BUS", "🚌", "Ônibus"),
    ic(2029, "CALLING", "📞", "Ligações"),
    ic(2030, "CANDY", "🍭", "Doces"),
    ic(2031, "CAR", "🚗", "Carro"),
    ic(2032, "CASH", "💵", "Dinheiro"),
    ic(2033, "CASINO", "🃏", "Cassino"),
    ic(2034, "TOOTH", "🦷", "Dentes"),
    ic(2035, "DISLIKE", "👎", "Não gosto"),
    ic(2036, "EMOTION_ANGRY", "😠", "Raiva"),
    ic(2037, "EMOTION_HAPPY", "😄", "Alegria"),
    ic(2038, "EMOTION_SAD", "😢", "Tristeza"),
    ic(2039, "EMOTION_NATURAL", "😐", "Neutro"),
    ic(2040, "GAMEPAD", "🕹️", "Fliperama"),
    ic(2041, "GOLFING", "⛳", "Golfe"),
    ic(2042, "HEADPHONES", "🎧", "Fones"),
    ic(2043, "LIKE", "👍", "Curtidas"),
    ic(2044, "MISSING_ALERTS", "🔕", "Notificações"),
    ic(2045, "MISSING_CALLS", "📵", "Celular demais"),
    ic(2046, "MUSIC", "🎵", "Música"),
    ic(2047, "NAPPING", "😴", "Cochilos"),
    ic(2048, "NOSE_PICKING", "👃", "Mania"),
    ic(2049, "OVERSLEEP", "⏰", "Dormir demais"),
    ic(2050, "PHONE", "📱", "Celular"),
    ic(2051, "PIZZA", "🍕", "Pizza"),
    ic(2052, "SHOPPING_CART", "🛒", "Mercado"),
    ic(2053, "HEART_BROKEN", "💔", "Coração partido"),
    ic(2054, "BEER_MEAL", "🍻", "Cerveja"),
    ic(2055, "GOAL", "🎯", "Objetivo"),
    ic(2056, "TIKTOK", "🎶", "TikTok"),
    ic(2057, "VAPING", "💨", "Vape"),
    ic(2058, "SNUS", "👄", "Tabaco de mascar"),
    ic(2059, "SELF_HARM", "🩹", "Autolesão"),
    // Hábitos para criar (códigos próprios do UnLoop).
    ic(3001, "WAKE_EARLY", "🌅", "Acordar cedo"),
    ic(3002, "EXERCISE", "🏃", "Exercício"),
    ic(3003, "READING", "📖", "Leitura"),
    ic(3004, "MEDITATION", "🧘", "Meditação"),
    ic(3005, "WATER", "💧", "Beber água"),
    ic(3006, "STUDY", "🎓", "Estudar"),
    ic(3007, "JOURNALING", "✍️", "Escrever no diário"),
    ic(3008, "SLEEP_EARLY", "🛌", "Dormir cedo"),
    ic(3009, "STRETCHING", "🤸", "Alongamento"),
    ic(3010, "HEALTHY_FOOD", "🥗", "Comer bem"),
    ic(3011, "LANGUAGE", "🗣️", "Praticar idioma"),
    ic(3012, "GRATITUDE", "🙏", "Gratidão"),
    ic(3013, "WALK", "🚶", "Caminhada"),
    ic(3014, "CLEANING", "🧹", "Organizar a casa"),
];

pub fn icon(code: i64) -> &'static IconInfo {
    ICONS.iter().find(|i| i.code == code).unwrap_or(&ICONS[0])
}

pub fn icon_by_legacy(name: &str) -> &'static IconInfo {
    ICONS.iter().find(|i| i.legacy == name).unwrap_or(&ICONS[0])
}

// ----------------------------------------------------------------- Cores

#[derive(Debug)]
pub struct ColorInfo {
    pub code: i64,
    pub legacy: &'static str,
    /// "RRGGBB"
    pub hex: &'static str,
    pub label: &'static str,
}

const fn co(code: i64, legacy: &'static str, hex: &'static str, label: &'static str) -> ColorInfo {
    ColorInfo { code, legacy, hex, label }
}

pub const COLORS: &[ColorInfo] = &[
    co(1001, "RED", "E53935", "Vermelho"),
    co(1013, "SCARLET", "C62828", "Escarlate"),
    co(1004, "PINK", "D81B60", "Rosa"),
    co(1010, "ORANGE", "FB8C00", "Laranja"),
    co(1014, "SALMON", "F07B6B", "Salmão"),
    co(1009, "YELLOW", "F9A825", "Amarelo"),
    co(1002, "GREEN", "43A047", "Verde"),
    co(1008, "TEAL", "00897B", "Verde-azulado"),
    co(1007, "CYAN", "00ACC1", "Ciano"),
    co(1003, "BLUE", "1E88E5", "Azul"),
    co(1015, "ROYAL_BLUE", "3F5FD8", "Azul royal"),
    co(1016, "BLUE_VIOLET", "7B3FD8", "Azul-violeta"),
    co(1017, "LAVENDER", "9575CD", "Lavanda"),
    co(1006, "INDIGO", "3949AB", "Índigo"),
    co(1018, "NAVY", "283593", "Azul-marinho"),
    co(1005, "PURPLE", "8E24AA", "Roxo"),
    co(1019, "MAUVE", "A87BA5", "Malva"),
    co(1020, "PLUM", "7E3F78", "Ameixa"),
    co(1021, "GRAY", "757575", "Cinza"),
    co(1011, "BROWN", "6D4C41", "Marrom"),
    co(1012, "BLUE_GRAY", "546E7A", "Cinza-azulado"),
];

pub const DEFAULT_COLOR: i64 = 1008;

pub fn color(code: i64) -> &'static ColorInfo {
    COLORS
        .iter()
        .find(|c| c.code == code)
        .unwrap_or_else(|| COLORS.iter().find(|c| c.code == DEFAULT_COLOR).unwrap_or(&COLORS[0]))
}

pub fn color_by_legacy(name: &str) -> &'static ColorInfo {
    COLORS.iter().find(|c| c.legacy == name).unwrap_or_else(|| color(DEFAULT_COLOR))
}

/// Normaliza "#rrggbb", "rrggbb" ou "aarrggbb" para "RRGGBB".
pub fn normalize_hex(hex: &str, fallback: &str) -> String {
    let clean: String = hex.trim().trim_start_matches('#').to_uppercase();
    let valid = !clean.is_empty() && clean.chars().all(|c| c.is_ascii_hexdigit());
    match (valid, clean.len()) {
        (true, 6) => clean,
        (true, 8) => clean[2..].to_string(),
        _ => fallback.to_string(),
    }
}

// -------------------------------------------------------------- Entidades

#[derive(Clone, Debug, PartialEq)]
pub struct Habit {
    pub id: i64,
    pub name: String,
    /// Código em [`ICONS`].
    pub icon: i64,
    /// Código em [`COLORS`].
    pub color: i64,
    /// Início da jornada ("última vez"), epoch ms.
    pub quit_at: i64,
    /// Dinheiro por dia.
    pub daily_money: f64,
    /// Minutos por dia.
    pub daily_minutes: f64,
    /// Vezes por dia.
    pub daily_frequency: f64,
    pub habit_since_ms: Option<i64>,
    pub preferred_metric: MetricType,
    pub counter_type: CounterType,
    pub counter_style: CounterStyle,
    pub position: i64,
    pub created_at: i64,
    /// Parar (contador) ou criar (check diário).
    pub kind: HabitKind,
    /// Dias programados para hábitos de check (bit 0 = segunda … bit 6 = domingo).
    pub schedule_days: u8,
    /// Meta diária de hábitos de check (ex.: 30 páginas). 0 = só marcar feito.
    pub daily_target: f64,
    /// Unidade da meta diária (ex.: "páginas", "min").
    pub target_unit: String,
}

impl Habit {
    pub fn new(name: impl Into<String>, quit_at: i64) -> Self {
        Habit {
            id: 0,
            name: name.into(),
            icon: ICONS[0].code,
            color: DEFAULT_COLOR,
            quit_at,
            daily_money: 0.0,
            daily_minutes: 0.0,
            daily_frequency: 0.0,
            habit_since_ms: None,
            preferred_metric: MetricType::Money,
            counter_type: CounterType::Abstinence,
            counter_style: CounterStyle::Full,
            position: 0,
            created_at: 0,
            kind: HabitKind::Quit,
            schedule_days: crate::checkin::ALL_DAYS,
            daily_target: 0.0,
            target_unit: String::new(),
        }
    }

    /// Hábito de check com meta diária (só conta o dia ao atingir a meta).
    pub fn has_target(&self) -> bool {
        self.is_build() && self.daily_target > 0.0
    }

    pub fn is_build(&self) -> bool {
        self.kind == HabitKind::Build
    }

    pub fn icon_info(&self) -> &'static IconInfo {
        icon(self.icon)
    }

    pub fn color_info(&self) -> &'static ColorInfo {
        color(self.color)
    }
}

/// Entrada do diário. `is_reset` = recaída (reinicia o contador).
#[derive(Clone, Debug, PartialEq)]
pub struct DiaryEntry {
    pub id: i64,
    pub habit_id: i64,
    pub occurred_at: i64,
    pub money_spent: f64,
    pub time_spent_ms: i64,
    pub is_reset: bool,
    pub note: Option<String>,
    /// Regra (permitido/não permitido) a que a entrada se refere, se houver.
    pub rule_id: Option<i64>,
}

impl DiaryEntry {
    pub fn reset(habit_id: i64, occurred_at: i64) -> Self {
        DiaryEntry {
            id: 0,
            habit_id,
            occurred_at,
            money_spent: 0.0,
            time_spent_ms: 0,
            is_reset: true,
            note: None,
            rule_id: None,
        }
    }

    pub fn has_note(&self) -> bool {
        self.note.as_deref().is_some_and(|n| !n.trim().is_empty())
    }
}

/// Meta. Padrão: `duration_ms` relativo ao início da sequência. Pessoal: `target_at` absoluto.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: i64,
    pub habit_id: i64,
    pub name: String,
    pub preset_key: Option<String>,
    pub duration_ms: Option<i64>,
    pub target_at: Option<i64>,
    /// "RRGGBB"
    pub hex_color: String,
    pub is_personal: bool,
    pub is_forced_current: bool,
}

impl Goal {
    pub fn preset(habit_id: i64, name: &str, key: &str, duration_ms: i64, hex: &str) -> Self {
        Goal {
            id: 0,
            habit_id,
            name: name.to_string(),
            preset_key: Some(key.to_string()),
            duration_ms: Some(duration_ms),
            target_at: None,
            hex_color: hex.to_string(),
            is_personal: false,
            is_forced_current: false,
        }
    }

    pub fn personal(habit_id: i64, name: &str, target_at: i64, hex: &str) -> Self {
        Goal {
            id: 0,
            habit_id,
            name: name.to_string(),
            preset_key: None,
            duration_ms: None,
            target_at: Some(target_at),
            hex_color: hex.to_string(),
            is_personal: true,
            is_forced_current: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reward {
    pub id: i64,
    pub habit_id: i64,
    pub name: String,
    pub price: f64,
    pub purchased_at: Option<i64>,
    /// Removida da lista; se comprada, continua contando no saldo.
    pub archived: bool,
    pub position: i64,
}

impl Reward {
    pub fn new(habit_id: i64, name: &str, price: f64) -> Self {
        Reward {
            id: 0,
            habit_id,
            name: name.to_string(),
            price,
            purchased_at: None,
            archived: false,
            position: 0,
        }
    }

    pub fn is_purchased(&self) -> bool {
        self.purchased_at.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeInvestment {
    pub id: i64,
    pub habit_id: i64,
    pub name: String,
    pub session_ms: i64,
    /// Soma das sessões investidas (continua contando se arquivado).
    pub invested_ms: i64,
    pub archived: bool,
    pub position: i64,
}

impl TimeInvestment {
    pub fn new(habit_id: i64, name: &str, session_ms: i64) -> Self {
        TimeInvestment {
            id: 0,
            habit_id,
            name: name.to_string(),
            session_ms,
            invested_ms: 0,
            archived: false,
            position: 0,
        }
    }
}

/// Regra do hábito. Ex. (parar com refrigerante): "Refrigerante zero" = permitido,
/// "Refrigerante comum" = não permitido.
#[derive(Clone, Debug, PartialEq)]
pub struct HabitRule {
    pub id: i64,
    pub habit_id: i64,
    pub kind: RuleKind,
    pub text: String,
    pub position: i64,
}

/// Progresso de um dia num hábito de check. Sem meta, `amount` = 1 (feito).
#[derive(Clone, Debug, PartialEq)]
pub struct DayProgress {
    /// "AAAA-MM-DD" (data local)
    pub day: String,
    pub amount: f64,
}

/// Motivo para parar ou afirmação.
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub id: i64,
    pub habit_id: i64,
    pub text: String,
    pub position: i64,
}
