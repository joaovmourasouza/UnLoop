//! Metas padrão, sugestões de hábitos, afirmações iniciais e moedas.

use std::sync::OnceLock;

use crate::units::{MS_PER_DAY, MS_PER_MONTH, MS_PER_WEEK, MS_PER_YEAR};

#[derive(Clone, Debug, PartialEq)]
pub struct GoalPreset {
    pub key: String,
    pub ms: i64,
    pub label: String,
    /// "RRGGBB"
    pub hex: String,
}

/// 45 metas padrão ordenadas por duração, com cor em gradiente
/// (vermelho → roxo → azul → verde).
pub fn goal_presets() -> &'static [GoalPreset] {
    static PRESETS: OnceLock<Vec<GoalPreset>> = OnceLock::new();
    PRESETS.get_or_init(build_presets)
}

fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn build_presets() -> Vec<GoalPreset> {
    let mut raw: Vec<(String, i64, String)> = Vec::new();
    for d in [1, 2, 3, 4, 5, 6, 10, 50, 100, 200, 250, 300, 400, 500, 700, 1000, 2000, 3000] {
        raw.push((format!("DAY_{d}"), d * MS_PER_DAY, plural(d, "dia", "dias")));
    }
    for w in [1, 2, 3, 5, 6, 10] {
        raw.push((format!("WEEK_{w}"), w * MS_PER_WEEK, plural(w, "semana", "semanas")));
    }
    for m in [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 15] {
        raw.push((format!("MONTH_{m}"), m * MS_PER_MONTH, plural(m, "mês", "meses")));
    }
    for y in 1..=10 {
        raw.push((format!("YEAR_{y}"), y * MS_PER_YEAR, plural(y, "ano", "anos")));
    }
    raw.sort_by_key(|r| r.1);

    let last = (raw.len() - 1) as f64;
    raw.into_iter()
        .enumerate()
        .map(|(i, (key, ms, label))| GoalPreset {
            key,
            ms,
            label,
            hex: hsv_to_hex(355.0 - i as f64 * 205.0 / last, 0.62, 0.86),
        })
        .collect()
}

pub fn preset_by_key(key: &str) -> Option<&'static GoalPreset> {
    goal_presets().iter().find(|p| p.key == key)
}

/// Preset mais próximo de `ms` (tolerância de 12 horas).
pub fn preset_for_duration(ms: i64) -> Option<&'static GoalPreset> {
    goal_presets()
        .iter()
        .filter(|p| (p.ms - ms).abs() <= MS_PER_DAY / 2)
        .min_by_key(|p| (p.ms - ms).abs())
}

pub fn hsv_to_hex(hue: f64, saturation: f64, value: f64) -> String {
    let h = hue.rem_euclid(360.0);
    let c = value * saturation;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let ch = |v: f64| (((v + m) * 255.0).round() as i64).clamp(0, 255);
    format!("{:02X}{:02X}{:02X}", ch(r), ch(g), ch(b))
}

/// Códigos de ícone sugeridos ao criar um hábito.
pub const HABIT_SUGGESTIONS: &[i64] = &[
    2002, 2057, 2003, 2006, 2004, 2005, 2009, 2016, 2010, 2011, 2018, 2056, 2015, 2013, 2014,
    2017, 2012, 2007, 2008, 20120, 2021, 2019, 2024, 2022, 2023, 2025, 2026, 2027,
];

pub const DEFAULT_AFFIRMATIONS: &[&str] = &[
    "Eu estou no controle das minhas escolhas.",
    "Cada dia limpo é uma vitória que ninguém tira de mim.",
    "Eu escolho a minha liberdade.",
];

pub const CURRENCY_CODES: &[&str] = &[
    "BRL", "USD", "EUR", "GBP", "ARS", "CLP", "MXN", "CAD", "AUD", "NZD", "CHF", "JPY", "CNY",
    "HKD", "TWD", "KRW", "INR", "IDR", "MYR", "PHP", "SGD", "THB", "VND", "PKR", "AED", "SAR",
    "BHD", "OMR", "IQD", "IRR", "ILS", "EGP", "MAD", "NGN", "ZAR", "TRY", "PLN", "CZK", "HUF",
    "RON", "BGN", "HRK", "DKK", "NOK", "SEK", "UAH", "BYN", "RUB", "KZT", "AZN", "VES",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metas_padrao() {
        let p = goal_presets();
        assert_eq!(p.len(), 45);
        assert!(p.windows(2).all(|w| w[1].ms > w[0].ms));
        let keys: std::collections::HashSet<_> = p.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys.len(), 45);
        assert_eq!(p[0].label, "1 dia");
        assert_eq!(p[44].label, "10 anos");
        assert!(p.iter().all(|x| x.hex.len() == 6));
    }

    #[test]
    fn reconhece_duracoes_do_quitzilla() {
        assert_eq!(preset_for_duration(5_260_000_000).map(|p| p.key.as_str()), Some("MONTH_2"));
        assert_eq!(preset_for_duration(7_889_238_000).map(|p| p.key.as_str()), Some("MONTH_3"));
        assert_eq!(preset_for_duration(86_400_000).map(|p| p.key.as_str()), Some("DAY_1"));
        assert_eq!(preset_for_duration(123), None);
    }

    #[test]
    fn cores() {
        assert_eq!(hsv_to_hex(0.0, 1.0, 1.0), "FF0000");
        assert_eq!(hsv_to_hex(120.0, 1.0, 1.0), "00FF00");
        assert_eq!(crate::model::normalize_hex("#0080e0", "757575"), "0080E0");
        assert_eq!(crate::model::normalize_hex("FF0080E0", "757575"), "0080E0");
        assert_eq!(crate::model::normalize_hex("xyz", "757575"), "757575");
    }
}
