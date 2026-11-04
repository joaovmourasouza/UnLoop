//! Padrões nos dados de um hábito, para relatórios e para o Claude analisar.

use chrono::{Datelike, Timelike};

use crate::calc::HabitBundle;
use crate::checkin::{local_date, WEEKDAY_SHORT};
use crate::model::DiaryEntry;

fn local_hour(ms: i64) -> usize {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).hour() as usize)
        .unwrap_or(0)
}

/// Recaídas por dia da semana (0 = segunda).
pub fn relapses_by_weekday(entries: &[DiaryEntry]) -> [u32; 7] {
    let mut out = [0; 7];
    for e in entries.iter().filter(|e| e.is_reset) {
        out[local_date(e.occurred_at).weekday().num_days_from_monday() as usize] += 1;
    }
    out
}

/// Recaídas por hora do dia.
pub fn relapses_by_hour(entries: &[DiaryEntry]) -> [u32; 24] {
    let mut out = [0; 24];
    for e in entries.iter().filter(|e| e.is_reset) {
        out[local_hour(e.occurred_at) % 24] += 1;
    }
    out
}

/// Faixa do dia: madrugada (0–5), manhã (6–11), tarde (12–17), noite (18–23).
pub fn relapses_by_period_of_day(entries: &[DiaryEntry]) -> [(&'static str, u32); 4] {
    let hours = relapses_by_hour(entries);
    let sum = |range: std::ops::Range<usize>| hours[range].iter().sum::<u32>();
    [("madrugada", sum(0..6)), ("manhã", sum(6..12)), ("tarde", sum(12..18)), ("noite", sum(18..24))]
}

/// Dia da semana com mais recaídas (se houver alguma).
pub fn riskiest_weekday(entries: &[DiaryEntry]) -> Option<&'static str> {
    let by_day = relapses_by_weekday(entries);
    let (index, count) = by_day.iter().enumerate().max_by_key(|(_, c)| **c)?;
    (*count > 0).then(|| WEEKDAY_SHORT[index])
}

/// Resumo em texto simples (útil na CLI e como ponto de partida para análises).
pub fn summary_lines(bundle: &HabitBundle, now: i64) -> Vec<String> {
    let mut lines = Vec::new();
    let entries = &bundle.entries;
    let resets = entries.iter().filter(|e| e.is_reset).count();
    if bundle.habit.is_build() {
        let s = bundle.checkin_stats(crate::checkin::today());
        lines.push(format!(
            "Sequência atual: {} · melhor: {} · conclusão: {:.0}%",
            s.current_streak,
            s.best_streak,
            s.completion_rate() * 100.0
        ));
    } else {
        let snap = bundle.snapshot(now);
        lines.push(format!(
            "Sequência atual: {} · recaídas: {resets}",
            crate::format::format_compact(snap.elapsed())
        ));
        if let Some(avg) = snap.periods.average() {
            lines.push(format!("Média dos períodos sem recaída: {}", crate::format::format_compact(avg)));
        }
        if let Some(day) = riskiest_weekday(entries) {
            lines.push(format!("Dia com mais recaídas: {day}"));
        }
        let periods = relapses_by_period_of_day(entries);
        if let Some((name, count)) = periods.iter().max_by_key(|(_, c)| *c).filter(|(_, c)| *c > 0) {
            lines.push(format!("Faixa do dia com mais recaídas: {name} ({count})"));
        }
    }
    let with_notes = entries.iter().filter(|e| e.has_note()).count();
    lines.push(format!("Entradas no diário: {} ({} com anotação)", entries.len(), with_notes));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::local_ms;
    use chrono::NaiveDate;

    fn at(day: u32, hour: u32) -> i64 {
        local_ms(NaiveDate::from_ymd_opt(2025, 1, day).unwrap(), hour, 0)
    }

    #[test]
    fn padroes_de_recaida() {
        let entries = vec![
            DiaryEntry::reset(1, at(17, 15)), // sexta à tarde
            DiaryEntry::reset(1, at(24, 16)), // sexta à tarde
            DiaryEntry::reset(1, at(20, 22)), // segunda à noite
            DiaryEntry { is_reset: false, ..DiaryEntry::reset(1, at(21, 9)) }, // anotação: não conta
        ];
        let by_day = relapses_by_weekday(&entries);
        assert_eq!(by_day[4], 2);
        assert_eq!(by_day[0], 1);
        assert_eq!(by_day.iter().sum::<u32>(), 3);
        assert_eq!(riskiest_weekday(&entries), Some("Sex"));
        let hours = relapses_by_hour(&entries);
        assert_eq!(hours[15] + hours[16], 2);
        assert_eq!(relapses_by_period_of_day(&entries)[2], ("tarde", 2));
        assert_eq!(riskiest_weekday(&[]), None);
    }
}
