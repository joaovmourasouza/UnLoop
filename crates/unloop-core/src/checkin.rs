//! Hábitos de check diário ("criar um hábito"): dias programados, sequência,
//! melhor sequência, taxa de conclusão e troféus.
//!
//! Regras:
//! - Só os dias programados (máscara de dias da semana) contam para a sequência.
//! - Dia programado sem check quebra a sequência, exceto hoje (o dia ainda não acabou).
//! - Check em dia não programado é bônus: entra no total, mas não altera a sequência.

use std::collections::{BTreeMap, BTreeSet};

pub use chrono::{Datelike, Days, NaiveDate, Weekday};

use crate::model::{DayProgress, Trophy};
use crate::units::MS_PER_DAY;

/// Bit 0 = segunda … bit 6 = domingo.
pub const ALL_DAYS: u8 = 0b111_1111;
pub const WEEKDAYS: u8 = 0b001_1111;
pub const WEEKEND: u8 = 0b110_0000;

pub const WEEKDAY_SHORT: [&str; 7] = ["Seg", "Ter", "Qua", "Qui", "Sex", "Sáb", "Dom"];
pub const WEEKDAY_LETTER: [&str; 7] = ["S", "T", "Q", "Q", "S", "S", "D"];

pub fn normalize_mask(mask: i64) -> u8 {
    let m = (mask & ALL_DAYS as i64) as u8;
    if m == 0 {
        ALL_DAYS
    } else {
        m
    }
}

pub fn is_scheduled(mask: u8, day: NaiveDate) -> bool {
    mask & (1 << day.weekday().num_days_from_monday()) != 0
}

/// "Seg, Ter, Qua" / "Todos os dias" / "Dias úteis" / "Fins de semana".
pub fn describe_mask(mask: u8) -> String {
    match mask {
        ALL_DAYS => "Todos os dias".into(),
        WEEKDAYS => "Segunda a sexta".into(),
        WEEKEND => "Fins de semana".into(),
        _ => (0..7)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| WEEKDAY_SHORT[i])
            .collect::<Vec<_>>()
            .join(", "),
    }
}

pub fn day_key(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

pub fn parse_day(key: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(key.trim(), "%Y-%m-%d").ok()
}

/// Data local de hoje.
pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// "AAAA-MM-DD HH:MM" no fuso local.
pub fn local_datetime_string(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Data local de um instante (epoch ms).
pub fn local_date(ms: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_else(today)
}

pub fn parse_days(keys: &[String]) -> BTreeSet<NaiveDate> {
    keys.iter().filter_map(|k| parse_day(k)).collect()
}

/// Um dia conta como feito quando o progresso atinge a meta (sem meta: qualquer registro).
pub fn day_done(amount: f64, target: f64) -> bool {
    if target > 0.0 {
        amount + 1e-9 >= target
    } else {
        amount > 0.0
    }
}

/// Dias que contam como feitos.
pub fn done_days(progress: &[DayProgress], target: f64) -> BTreeSet<NaiveDate> {
    progress
        .iter()
        .filter(|p| day_done(p.amount, target))
        .filter_map(|p| parse_day(&p.day))
        .collect()
}

/// Progresso registrado num dia (0 se nada).
pub fn amount_on(progress: &[DayProgress], day: NaiveDate) -> f64 {
    let key = day_key(day);
    progress.iter().find(|p| p.day == key).map_or(0.0, |p| p.amount)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckinStats {
    pub current_streak: u32,
    pub best_streak: u32,
    /// Checks entre o início e hoje (inclusive bônus em dias não programados).
    pub total_checks: u32,
    /// Dias programados já "vencidos" (hoje só conta se já foi feito).
    pub scheduled_days: u32,
    pub completed_scheduled: u32,
    pub scheduled_today: bool,
    pub done_today: bool,
    /// Código do troféu → dia da conquista.
    pub trophies: BTreeMap<i64, Option<NaiveDate>>,
}

impl CheckinStats {
    pub fn completion_rate(&self) -> f64 {
        if self.scheduled_days == 0 {
            0.0
        } else {
            self.completed_scheduled as f64 / self.scheduled_days as f64
        }
    }

    pub fn trophy_at(&self, t: Trophy) -> Option<NaiveDate> {
        self.trophies.get(&t.code()).copied().flatten()
    }

    pub fn trophies_achieved(&self) -> usize {
        self.trophies.values().filter(|v| v.is_some()).count()
    }
}

/// Limiar do troféu em dias programados seguidos (DAY_180 = 182).
pub fn trophy_days(t: Trophy) -> u32 {
    (t.ms() / MS_PER_DAY) as u32
}

pub fn checkin_stats(start: NaiveDate, mask: u8, checks: &BTreeSet<NaiveDate>, today: NaiveDate) -> CheckinStats {
    let mask = if mask == 0 { ALL_DAYS } else { mask };
    let mut stats = CheckinStats {
        current_streak: 0,
        best_streak: 0,
        total_checks: 0,
        scheduled_days: 0,
        completed_scheduled: 0,
        scheduled_today: is_scheduled(mask, today),
        done_today: checks.contains(&today),
        trophies: Trophy::ALL.iter().map(|t| (t.code(), None)).collect(),
    };
    if start > today {
        return stats;
    }
    stats.trophies.insert(Trophy::DecisionMade.code(), Some(start));

    let mut run: u32 = 0;
    let mut day = start;
    while day <= today {
        let checked = checks.contains(&day);
        if checked {
            stats.total_checks += 1;
        }
        if is_scheduled(mask, day) {
            if checked {
                run += 1;
                stats.completed_scheduled += 1;
                stats.scheduled_days += 1;
            } else if day != today {
                run = 0;
                stats.scheduled_days += 1;
            }
            stats.best_streak = stats.best_streak.max(run);
            for &t in Trophy::ALL {
                if t == Trophy::DecisionMade {
                    continue;
                }
                let slot = stats.trophies.entry(t.code()).or_insert(None);
                if slot.is_none() && run >= trophy_days(t) {
                    *slot = Some(day);
                }
            }
        }
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }
    stats.current_streak = run;
    stats
}

/// Dias da semana de `week_start` (segunda) até domingo.
pub fn week_days(week_start: NaiveDate) -> Vec<NaiveDate> {
    (0..7).filter_map(|i| week_start.checked_add_days(chrono::Days::new(i))).collect()
}

/// Segunda-feira da semana de `day`.
pub fn monday_of(day: NaiveDate) -> NaiveDate {
    day.checked_sub_days(chrono::Days::new(day.weekday().num_days_from_monday() as u64))
        .unwrap_or(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, day).unwrap()
    }

    fn set(days: &[u32]) -> BTreeSet<NaiveDate> {
        days.iter().map(|&x| d(x)).collect()
    }

    // 2025-01-06 é segunda-feira.
    #[test]
    fn dias_programados() {
        assert!(is_scheduled(WEEKDAYS, d(6)));
        assert!(!is_scheduled(WEEKDAYS, d(11))); // sábado
        assert!(!is_scheduled(WEEKDAYS, d(12))); // domingo
        assert!(is_scheduled(WEEKEND, d(12)));
        assert_eq!(describe_mask(WEEKDAYS), "Segunda a sexta");
        assert_eq!(describe_mask(0b000_0101), "Seg, Qua");
        assert_eq!(normalize_mask(0), ALL_DAYS);
        assert_eq!(monday_of(d(12)), d(6));
        assert_eq!(week_days(d(6)).len(), 7);
        assert_eq!(day_key(d(6)), "2025-01-06");
        assert_eq!(parse_day("2025-01-06"), Some(d(6)));
    }

    #[test]
    fn fim_de_semana_nao_quebra_e_hoje_pendente_nao_quebra() {
        let s = checkin_stats(d(6), WEEKDAYS, &set(&[6, 7, 8, 9, 10, 13, 14]), d(15));
        assert_eq!(s.current_streak, 7);
        assert_eq!(s.best_streak, 7);
        assert_eq!(s.scheduled_days, 7);
        assert_eq!(s.completed_scheduled, 7);
        assert!((s.completion_rate() - 1.0).abs() < 1e-9);
        assert!(s.scheduled_today);
        assert!(!s.done_today);
        assert_eq!(s.trophy_at(Trophy::DecisionMade), Some(d(6)));
        assert_eq!(s.trophy_at(Trophy::Day1), Some(d(6)));
        assert_eq!(s.trophy_at(Trophy::Day3), Some(d(8)));
        assert_eq!(s.trophy_at(Trophy::Day7), Some(d(14)));
        assert_eq!(s.trophy_at(Trophy::Day10), None);
    }

    #[test]
    fn dia_programado_sem_check_quebra() {
        let s = checkin_stats(d(6), WEEKDAYS, &set(&[6, 7, 8, 10, 13, 14]), d(15));
        assert_eq!(s.current_streak, 3);
        assert_eq!(s.best_streak, 3);
        assert_eq!(s.scheduled_days, 7);
        assert_eq!(s.completed_scheduled, 6);
    }

    #[test]
    fn check_de_hoje_e_bonus_no_fim_de_semana() {
        let s = checkin_stats(d(6), WEEKDAYS, &set(&[6, 7, 8, 9, 10, 11, 13, 14, 15]), d(15));
        assert_eq!(s.current_streak, 8);
        assert_eq!(s.scheduled_days, 8);
        assert_eq!(s.total_checks, 9); // sábado conta como bônus
        assert!(s.done_today);
    }

    #[test]
    fn ontem_sem_check_zera_a_sequencia_atual() {
        let s = checkin_stats(d(6), ALL_DAYS, &set(&[6, 7, 8]), d(10));
        assert_eq!(s.current_streak, 0);
        assert_eq!(s.best_streak, 3);
    }

    #[test]
    fn meta_diaria() {
        let progress = vec![
            DayProgress { day: "2025-01-06".into(), amount: 30.0 },
            DayProgress { day: "2025-01-07".into(), amount: 12.0 }, // abaixo da meta
            DayProgress { day: "2025-01-08".into(), amount: 45.0 },
        ];
        assert!(day_done(30.0, 30.0));
        assert!(!day_done(29.9, 30.0));
        assert!(day_done(1.0, 0.0));
        let done = done_days(&progress, 30.0);
        assert_eq!(done, set(&[6, 8]));
        assert_eq!(amount_on(&progress, d(7)), 12.0);
        assert_eq!(amount_on(&progress, d(9)), 0.0);
        // Dia 7 não bateu a meta: a sequência quebra.
        let s = checkin_stats(d(6), ALL_DAYS, &done, d(8));
        assert_eq!(s.current_streak, 1);
        assert_eq!(s.best_streak, 1);
        // Sem meta, qualquer registro conta.
        assert_eq!(done_days(&progress, 0.0).len(), 3);
    }

    #[test]
    fn inicio_no_futuro() {
        let s = checkin_stats(d(20), ALL_DAYS, &set(&[]), d(10));
        assert_eq!(s.current_streak, 0);
        assert_eq!(s.trophies_achieved(), 0);
    }
}
