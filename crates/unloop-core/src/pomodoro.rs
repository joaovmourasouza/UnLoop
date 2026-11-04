//! Timer de foco (Pomodoro e cronômetro), regras de crédito em hábitos e
//! resumo das sessões. Sem I/O, exceto `Settings::load/save`.

use crate::checkin::{local_date, NaiveDate};
use crate::db::Db;
use crate::model::Habit;
use crate::units::{MS_PER_DAY, MS_PER_MINUTE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Foco",
            Phase::ShortBreak => "Pausa curta",
            Phase::LongBreak => "Pausa longa",
        }
    }

    pub fn is_focus(self) -> bool {
        self == Phase::Focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Pomodoro,
    Stopwatch,
}

impl Mode {
    pub fn code(self) -> i64 {
        match self {
            Mode::Pomodoro => 0,
            Mode::Stopwatch => 1,
        }
    }

    pub fn from_code(code: i64) -> Self {
        if code == 1 {
            Mode::Stopwatch
        } else {
            Mode::Pomodoro
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    /// Pausa longa a cada N focos concluídos.
    pub long_break_every: u32,
    pub auto_start_breaks: bool,
    pub auto_start_focus: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            long_break_every: 4,
            auto_start_breaks: true,
            auto_start_focus: false,
        }
    }
}

impl Settings {
    pub fn duration_ms(&self, phase: Phase) -> i64 {
        let minutes = match phase {
            Phase::Focus => self.focus_min,
            Phase::ShortBreak => self.short_break_min,
            Phase::LongBreak => self.long_break_min,
        };
        i64::from(minutes.max(1)) * MS_PER_MINUTE
    }

    pub fn load(db: &Db) -> Settings {
        let d = Settings::default();
        let num = |key: &str, default: u32| {
            db.setting(key).ok().flatten().and_then(|v| v.parse::<u32>().ok()).filter(|v| *v > 0).unwrap_or(default)
        };
        let flag = |key: &str, default: bool| db.setting(key).ok().flatten().map_or(default, |v| v == "1");
        Settings {
            focus_min: num("pomodoro_focus_min", d.focus_min),
            short_break_min: num("pomodoro_short_min", d.short_break_min),
            long_break_min: num("pomodoro_long_min", d.long_break_min),
            long_break_every: num("pomodoro_long_every", d.long_break_every),
            auto_start_breaks: flag("pomodoro_auto_breaks", d.auto_start_breaks),
            auto_start_focus: flag("pomodoro_auto_focus", d.auto_start_focus),
        }
    }

    pub fn save(&self, db: &Db) -> rusqlite::Result<()> {
        let b = |v: bool| if v { "1" } else { "0" };
        db.set_setting("pomodoro_focus_min", &self.focus_min.to_string())?;
        db.set_setting("pomodoro_short_min", &self.short_break_min.to_string())?;
        db.set_setting("pomodoro_long_min", &self.long_break_min.to_string())?;
        db.set_setting("pomodoro_long_every", &self.long_break_every.to_string())?;
        db.set_setting("pomodoro_auto_breaks", b(self.auto_start_breaks))?;
        db.set_setting("pomodoro_auto_focus", b(self.auto_start_focus))
    }
}

/// Sessão de foco concluída.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusRecord {
    pub id: i64,
    pub habit_id: Option<i64>,
    pub started_at: i64,
    pub ended_at: i64,
    pub duration_ms: i64,
    pub mode: Mode,
}

/// Sessões com menos de 1 minuto de foco não são registradas.
pub const MIN_RECORD_MS: i64 = MS_PER_MINUTE;

#[derive(Clone, Debug, PartialEq)]
pub struct Timer {
    pub mode: Mode,
    pub phase: Phase,
    /// Hábito que recebe o crédito do foco.
    pub habit_id: Option<i64>,
    /// Focos concluídos neste ciclo (define a pausa longa).
    pub focus_count: u32,
    running_since: Option<i64>,
    accumulated: i64,
    session_start: Option<i64>,
}

impl Default for Timer {
    fn default() -> Self {
        Timer {
            mode: Mode::Pomodoro,
            phase: Phase::Focus,
            habit_id: None,
            focus_count: 0,
            running_since: None,
            accumulated: 0,
            session_start: None,
        }
    }
}

impl Timer {
    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Parado e zerado (nada em andamento).
    pub fn is_idle(&self) -> bool {
        self.running_since.is_none() && self.accumulated == 0
    }

    pub fn elapsed(&self, now: i64) -> i64 {
        self.accumulated + self.running_since.map_or(0, |s| (now - s).max(0))
    }

    pub fn remaining(&self, now: i64, s: &Settings) -> i64 {
        (s.duration_ms(self.phase) - self.elapsed(now)).max(0)
    }

    /// Tempo exibido: regressivo no Pomodoro, progressivo no cronômetro.
    pub fn display_ms(&self, now: i64, s: &Settings) -> i64 {
        match self.mode {
            Mode::Pomodoro => self.remaining(now, s),
            Mode::Stopwatch => self.elapsed(now),
        }
    }

    pub fn progress(&self, now: i64, s: &Settings) -> f64 {
        match self.mode {
            Mode::Pomodoro => (self.elapsed(now) as f64 / s.duration_ms(self.phase) as f64).clamp(0.0, 1.0),
            Mode::Stopwatch => 0.0,
        }
    }

    pub fn start(&mut self, now: i64) {
        if self.running_since.is_none() {
            self.running_since = Some(now);
            if self.session_start.is_none() {
                self.session_start = Some(now);
            }
        }
    }

    pub fn pause(&mut self, now: i64) {
        if let Some(since) = self.running_since.take() {
            self.accumulated += (now - since).max(0);
        }
    }

    /// Troca de modo e zera tudo (mantém o hábito vinculado).
    pub fn set_mode(&mut self, mode: Mode) {
        *self = Timer { mode, habit_id: self.habit_id, ..Timer::default() };
    }

    fn partial_record(&self, now: i64) -> Option<FocusRecord> {
        let duration = self.elapsed(now);
        (self.phase.is_focus() && duration >= MIN_RECORD_MS).then(|| FocusRecord {
            id: 0,
            habit_id: self.habit_id,
            started_at: self.session_start.unwrap_or(now - duration),
            ended_at: now,
            duration_ms: duration,
            mode: self.mode,
        })
    }

    fn advance(&mut self, now: i64, s: &Settings, focus_completed: bool) {
        self.phase = if self.phase.is_focus() {
            if focus_completed {
                self.focus_count += 1;
            }
            if focus_completed && self.focus_count % s.long_break_every.max(1) == 0 {
                Phase::LongBreak
            } else {
                Phase::ShortBreak
            }
        } else {
            Phase::Focus
        };
        self.accumulated = 0;
        self.session_start = None;
        self.running_since = None;
        let auto = if self.phase.is_focus() { s.auto_start_focus } else { s.auto_start_breaks };
        if auto {
            self.start(now);
        }
    }

    /// Chamado a cada segundo. Quando uma fase do Pomodoro termina, avança e
    /// devolve a fase encerrada e, se era foco, o registro da sessão.
    pub fn tick(&mut self, now: i64, s: &Settings) -> Option<(Phase, Option<FocusRecord>)> {
        if self.mode != Mode::Pomodoro || !self.is_running() {
            return None;
        }
        let duration = s.duration_ms(self.phase);
        if self.elapsed(now) < duration {
            return None;
        }
        let finished = self.phase;
        let record = finished.is_focus().then(|| FocusRecord {
            id: 0,
            habit_id: self.habit_id,
            started_at: self.session_start.unwrap_or(now - duration),
            ended_at: now,
            duration_ms: duration,
            mode: Mode::Pomodoro,
        });
        self.advance(now, s, finished.is_focus());
        Some((finished, record))
    }

    /// Pula para a próxima fase. Um foco interrompido com 1 min ou mais é registrado.
    pub fn skip(&mut self, now: i64, s: &Settings) -> Option<FocusRecord> {
        let record = self.partial_record(now);
        self.advance(now, s, false);
        record
    }

    /// Encerra tudo e volta ao início do foco. Registra o foco feito até aqui (>= 1 min).
    pub fn stop(&mut self, now: i64) -> Option<FocusRecord> {
        let record = self.partial_record(now);
        *self = Timer { mode: self.mode, habit_id: self.habit_id, ..Timer::default() };
        record
    }
}

/// O que uma sessão de foco faz num hábito vinculado.
#[derive(Clone, Debug, PartialEq)]
pub enum Credit {
    /// Nada (hábito de parar, ou meta numa unidade que não é tempo).
    None,
    /// Soma ao progresso do dia (ex.: minutos de estudo).
    AddProgress(f64),
    /// Marca o dia como feito (hábito de check sem meta).
    Check,
}

pub fn focus_credit(habit: &Habit, duration_ms: i64) -> Credit {
    if !habit.is_build() {
        return Credit::None;
    }
    if habit.daily_target <= 0.0 {
        return Credit::Check;
    }
    let minutes = duration_ms as f64 / MS_PER_MINUTE as f64;
    let unit = habit.target_unit.trim().to_lowercase();
    match unit.as_str() {
        "min" | "mins" | "m" | "minuto" | "minutos" => Credit::AddProgress((minutes * 10.0).round() / 10.0),
        "h" | "hora" | "horas" => Credit::AddProgress((minutes / 60.0 * 100.0).round() / 100.0),
        "pomodoro" | "pomodoros" => Credit::AddProgress(1.0),
        _ => Credit::None,
    }
}

/// Resumo das sessões: hoje e os últimos 7 dias (do mais antigo para hoje).
#[derive(Clone, Debug, PartialEq)]
pub struct FocusSummary {
    pub today_ms: i64,
    pub today_sessions: usize,
    pub week: Vec<(NaiveDate, i64)>,
    pub week_ms: i64,
}

pub fn summarize(records: &[FocusRecord], today: NaiveDate) -> FocusSummary {
    let days: Vec<NaiveDate> = (0..7u64)
        .rev()
        .filter_map(|back| today.checked_sub_days(chrono::Days::new(back)))
        .collect();
    let mut week: Vec<(NaiveDate, i64)> = days.iter().map(|d| (*d, 0)).collect();
    let mut today_ms = 0;
    let mut today_sessions = 0;
    for r in records {
        let day = local_date(r.started_at);
        if let Some(slot) = week.iter_mut().find(|(d, _)| *d == day) {
            slot.1 += r.duration_ms;
        }
        if day == today {
            today_ms += r.duration_ms;
            today_sessions += 1;
        }
    }
    let week_ms = week.iter().map(|(_, ms)| ms).sum();
    FocusSummary { today_ms, today_sessions, week, week_ms }
}

/// Início da janela de 7 dias (para consultar o banco).
pub fn week_window_start(now: i64) -> i64 {
    now - 8 * MS_PER_DAY
}

// ------------------------------------------------------------ Estatísticas

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
    Year,
}

const MONTHS_SHORT: [&str; 12] = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];
const MONTHS_LONG: [&str; 12] = [
    "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro",
    "dezembro",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Sessões no modo Pomodoro (🍅).
    pub pomodoros: usize,
    pub sessions: usize,
    pub ms: i64,
}

pub fn totals<'a>(records: impl IntoIterator<Item = &'a FocusRecord>) -> Totals {
    let mut t = Totals::default();
    for r in records {
        t.sessions += 1;
        t.ms += r.duration_ms;
        if r.mode == Mode::Pomodoro {
            t.pomodoros += 1;
        }
    }
    t
}

fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let (y, m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 28).unwrap_or(NaiveDate::MIN))
}

/// Primeiro e último dia (inclusive) do período que contém `anchor`.
pub fn period_range(period: Period, anchor: NaiveDate) -> (NaiveDate, NaiveDate) {
    use chrono::Datelike;
    match period {
        Period::Week => {
            let start = crate::checkin::monday_of(anchor);
            (start, start.checked_add_days(chrono::Days::new(6)).unwrap_or(start))
        }
        Period::Month => (
            anchor.with_day(1).unwrap_or(anchor),
            last_day_of_month(anchor.year(), anchor.month()),
        ),
        Period::Year => (
            NaiveDate::from_ymd_opt(anchor.year(), 1, 1).unwrap_or(anchor),
            NaiveDate::from_ymd_opt(anchor.year(), 12, 31).unwrap_or(anchor),
        ),
    }
}

/// Avança/volta `delta` períodos.
pub fn shift_period(period: Period, anchor: NaiveDate, delta: i32) -> NaiveDate {
    use chrono::Months;
    let months = |n: i32| {
        if n >= 0 {
            anchor.checked_add_months(Months::new(n as u32))
        } else {
            anchor.checked_sub_months(Months::new(n.unsigned_abs()))
        }
    };
    let shifted = match period {
        Period::Week => {
            let days = chrono::Days::new(7 * u64::from(delta.unsigned_abs()));
            if delta >= 0 {
                anchor.checked_add_days(days)
            } else {
                anchor.checked_sub_days(days)
            }
        }
        Period::Month => months(delta),
        Period::Year => months(delta * 12),
    };
    shifted.unwrap_or(anchor)
}

pub fn period_label(period: Period, anchor: NaiveDate) -> String {
    use chrono::Datelike;
    let (start, end) = period_range(period, anchor);
    match period {
        Period::Week => format!(
            "{} {} – {} {} {}",
            start.day(),
            MONTHS_SHORT[start.month0() as usize],
            end.day(),
            MONTHS_SHORT[end.month0() as usize],
            end.year()
        ),
        Period::Month => format!("{} de {}", MONTHS_LONG[anchor.month0() as usize], anchor.year()),
        Period::Year => anchor.year().to_string(),
    }
}

fn in_range(r: &FocusRecord, (start, end): (NaiveDate, NaiveDate)) -> bool {
    let day = local_date(r.started_at);
    day >= start && day <= end
}

/// Sessões dentro do período.
pub fn in_period(records: &[FocusRecord], period: Period, anchor: NaiveDate) -> Vec<FocusRecord> {
    let range = period_range(period, anchor);
    records.iter().filter(|r| in_range(r, range)).cloned().collect()
}

/// Série do gráfico: (rótulo, ms). Semana: 7 dias; mês: cada dia; ano: 12 meses.
pub fn series(records: &[FocusRecord], period: Period, anchor: NaiveDate) -> Vec<(String, i64)> {
    use chrono::Datelike;
    let (start, end) = period_range(period, anchor);
    let selected: Vec<&FocusRecord> = records.iter().filter(|r| in_range(r, (start, end))).collect();
    match period {
        Period::Year => {
            let mut months = vec![0i64; 12];
            for r in selected {
                months[local_date(r.started_at).month0() as usize] += r.duration_ms;
            }
            MONTHS_SHORT.iter().zip(months).map(|(m, ms)| (m.to_string(), ms)).collect()
        }
        Period::Week | Period::Month => {
            let mut out = Vec::new();
            let mut day = start;
            while day <= end {
                let ms: i64 = selected.iter().filter(|r| local_date(r.started_at) == day).map(|r| r.duration_ms).sum();
                let label = if period == Period::Week {
                    crate::checkin::WEEKDAY_SHORT[day.weekday().num_days_from_monday() as usize].to_string()
                } else {
                    day.day().to_string()
                };
                out.push((label, ms));
                match day.succ_opt() {
                    Some(next) => day = next,
                    None => break,
                }
            }
            out
        }
    }
}

/// Tempo de foco por hábito (None = sem hábito), do maior para o menor.
pub fn by_habit(records: &[FocusRecord]) -> Vec<(Option<i64>, i64)> {
    let mut map: Vec<(Option<i64>, i64)> = Vec::new();
    for r in records {
        match map.iter_mut().find(|(h, _)| *h == r.habit_id) {
            Some(slot) => slot.1 += r.duration_ms,
            None => map.push((r.habit_id, r.duration_ms)),
        }
    }
    map.sort_by(|a, b| b.1.cmp(&a.1));
    map
}

/// Hora local (0–23) de um instante.
pub fn local_hour(ms: i64) -> u32 {
    use chrono::Timelike;
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).hour())
        .unwrap_or(0)
}

/// Tempo de foco por hora do dia em que a sessão começou.
pub fn by_hour(records: &[FocusRecord]) -> [i64; 24] {
    let mut hours = [0i64; 24];
    for r in records {
        hours[local_hour(r.started_at) as usize % 24] += r.duration_ms;
    }
    hours
}

/// Dias seguidos com pelo menos uma sessão, terminando hoje (ou ontem, se hoje ainda não teve).
/// Comparação de um período com o anterior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub current: Totals,
    pub previous: Totals,
    /// Período ainda em andamento: compara só até o mesmo ponto (ex.: seg–qua × seg–qua).
    pub partial: bool,
}

fn change(current: f64, previous: f64) -> Option<f64> {
    (previous > 0.0).then(|| (current - previous) / previous)
}

impl Comparison {
    /// Variação do tempo de foco (0,2 = +20%). `None` se o anterior foi zero.
    pub fn time_change(&self) -> Option<f64> {
        change(self.current.ms as f64, self.previous.ms as f64)
    }

    /// Variação do número de pomodoros.
    pub fn pomodoro_change(&self) -> Option<f64> {
        change(self.current.pomodoros as f64, self.previous.pomodoros as f64)
    }
}

pub fn compare_with_previous(records: &[FocusRecord], period: Period, anchor: NaiveDate, today: NaiveDate) -> Comparison {
    let (start, end) = period_range(period, anchor);
    let partial = today >= start && today < end;
    let current_end = if partial { today } else { end };
    let (prev_start, prev_full_end) = period_range(period, shift_period(period, anchor, -1));
    let prev_end = if partial {
        let offset = (current_end - start).num_days().max(0) as u64;
        prev_start.checked_add_days(chrono::Days::new(offset)).unwrap_or(prev_full_end).min(prev_full_end)
    } else {
        prev_full_end
    };
    Comparison {
        current: totals(records.iter().filter(|r| in_range(r, (start, current_end)))),
        previous: totals(records.iter().filter(|r| in_range(r, (prev_start, prev_end)))),
        partial,
    }
}

/// Ex.: "+20% em relação à semana anterior (até o mesmo dia)".
pub fn comparison_text(c: &Comparison, period: Period) -> String {
    let previous = match period {
        Period::Week => "à semana anterior",
        Period::Month => "ao mês anterior",
        Period::Year => "ao ano anterior",
    };
    let scope = if c.partial { " (até o mesmo dia)" } else { "" };
    match c.time_change() {
        None if c.current.ms == 0 => "Sem foco neste período nem no anterior.".to_string(),
        None => format!("Sem foco no período anterior para comparar{scope}."),
        Some(ch) => {
            let pct = (ch * 100.0).round() as i64;
            if pct == 0 {
                format!("Mesmo tempo de foco em relação {previous}{scope}.")
            } else {
                format!("{}{}% de foco em relação {previous}{scope}", if pct > 0 { "+" } else { "−" }, pct.abs())
            }
        }
    }
}

pub fn focus_day_streak(records: &[FocusRecord], today: NaiveDate) -> u32 {
    let days: std::collections::BTreeSet<NaiveDate> = records.iter().map(|r| local_date(r.started_at)).collect();
    let mut day = if days.contains(&today) {
        today
    } else {
        match today.pred_opt() {
            Some(d) => d,
            None => return 0,
        }
    };
    let mut streak = 0;
    while days.contains(&day) {
        streak += 1;
        match day.pred_opt() {
            Some(d) => day = d,
            None => break,
        }
    }
    streak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HabitKind;

    fn noon(y: i32, m: u32, d: u32) -> i64 {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap()
    }

    fn session(start: i64, minutes: i64, habit: Option<i64>, mode: Mode) -> FocusRecord {
        FocusRecord {
            id: 0,
            habit_id: habit,
            started_at: start,
            ended_at: start + minutes * MS_PER_MINUTE,
            duration_ms: minutes * MS_PER_MINUTE,
            mode,
        }
    }

    #[test]
    fn periodos() {
        let d = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(); // quarta
        assert_eq!(period_range(Period::Week, d).0, NaiveDate::from_ymd_opt(2025, 1, 13).unwrap());
        assert_eq!(period_range(Period::Week, d).1, NaiveDate::from_ymd_opt(2025, 1, 19).unwrap());
        assert_eq!(period_range(Period::Month, NaiveDate::from_ymd_opt(2024, 2, 10).unwrap()).1.day0() + 1, 29);
        assert_eq!(period_range(Period::Month, NaiveDate::from_ymd_opt(2025, 12, 3).unwrap()).1, NaiveDate::from_ymd_opt(2025, 12, 31).unwrap());
        assert_eq!(shift_period(Period::Week, d, -1), NaiveDate::from_ymd_opt(2025, 1, 8).unwrap());
        assert_eq!(shift_period(Period::Month, d, 1), NaiveDate::from_ymd_opt(2025, 2, 15).unwrap());
        assert_eq!(shift_period(Period::Year, d, -1), NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());
        assert_eq!(period_label(Period::Week, d), "13 jan – 19 jan 2025");
        assert_eq!(period_label(Period::Month, d), "janeiro de 2025");
        assert_eq!(period_label(Period::Year, d), "2025");
    }

    #[test]
    fn estatisticas_de_foco() {
        let records = vec![
            session(noon(2025, 1, 13), 25, Some(1), Mode::Pomodoro),
            session(noon(2025, 1, 13) + 30 * MS_PER_MINUTE, 25, Some(1), Mode::Pomodoro),
            session(noon(2025, 1, 14), 50, Some(2), Mode::Stopwatch),
            session(noon(2025, 1, 15), 25, None, Mode::Pomodoro),
            session(noon(2025, 2, 3), 25, Some(1), Mode::Pomodoro),
        ];
        let d = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();

        let all = totals(&records);
        assert_eq!(all.sessions, 5);
        assert_eq!(all.pomodoros, 4);
        assert_eq!(all.ms, 150 * MS_PER_MINUTE);

        let week = series(&records, Period::Week, d);
        assert_eq!(week.len(), 7);
        assert_eq!(week[0], ("Seg".to_string(), 50 * MS_PER_MINUTE));
        assert_eq!(week[1].1, 50 * MS_PER_MINUTE);
        assert_eq!(week[2].1, 25 * MS_PER_MINUTE);
        assert_eq!(week[3].1, 0);

        assert_eq!(series(&records, Period::Month, d).len(), 31);
        let year = series(&records, Period::Year, d);
        assert_eq!(year[0], ("jan".to_string(), 125 * MS_PER_MINUTE));
        assert_eq!(year[1].1, 25 * MS_PER_MINUTE);

        let in_week = in_period(&records, Period::Week, d);
        assert_eq!(in_week.len(), 4);
        let habits = by_habit(&in_week);
        assert_eq!(habits[0], (Some(1), 50 * MS_PER_MINUTE));
        assert_eq!(habits.len(), 3);

        let hours = by_hour(&records);
        assert_eq!(hours[12], 150 * MS_PER_MINUTE);
        assert_eq!(hours.iter().sum::<i64>(), 150 * MS_PER_MINUTE);

        assert_eq!(focus_day_streak(&records, d), 3);
        assert_eq!(focus_day_streak(&records, NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()), 3, "hoje sem foco ainda não quebra");
        assert_eq!(focus_day_streak(&records, NaiveDate::from_ymd_opt(2025, 1, 20).unwrap()), 0);
    }

    use chrono::Datelike;

    const T0: i64 = 1_735_689_600_000;
    const MIN: i64 = MS_PER_MINUTE;

    #[test]
    fn ciclo_completo_com_pausa_longa() {
        let s = Settings::default();
        let mut t = Timer::default();
        t.start(T0);
        assert!(t.tick(T0 + 10 * MIN, &s).is_none());
        assert_eq!(t.remaining(T0 + 10 * MIN, &s), 15 * MIN);

        let mut now = T0 + 25 * MIN;
        let (phase, record) = t.tick(now, &s).unwrap();
        assert_eq!(phase, Phase::Focus);
        assert_eq!(record.unwrap().duration_ms, 25 * MIN);
        assert_eq!(t.phase, Phase::ShortBreak);
        assert!(t.is_running(), "pausa começa sozinha por padrão");

        // Completa mais 3 focos: o 4º leva à pausa longa.
        for i in 0..3 {
            now += 5 * MIN;
            assert_eq!(t.tick(now, &s).unwrap().0, Phase::ShortBreak);
            assert_eq!(t.phase, Phase::Focus);
            assert!(!t.is_running(), "foco não começa sozinho por padrão");
            t.start(now);
            now += 25 * MIN;
            t.tick(now, &s).unwrap();
            let expected = if i == 2 { Phase::LongBreak } else { Phase::ShortBreak };
            assert_eq!(t.phase, expected);
        }
        assert_eq!(t.focus_count, 4);
    }

    #[test]
    fn pausar_e_retomar() {
        let s = Settings::default();
        let mut t = Timer::default();
        t.start(T0);
        t.pause(T0 + 5 * MIN);
        assert_eq!(t.elapsed(T0 + 60 * MIN), 5 * MIN);
        t.start(T0 + 60 * MIN);
        assert_eq!(t.elapsed(T0 + 62 * MIN), 7 * MIN);
        assert!(t.tick(T0 + 62 * MIN, &s).is_none());
    }

    #[test]
    fn encerrar_e_pular() {
        let s = Settings::default();
        let mut t = Timer { habit_id: Some(9), ..Timer::default() };
        t.start(T0);
        assert!(t.stop(T0 + 30_000).is_none(), "menos de 1 min não registra");
        assert!(t.is_idle());
        assert_eq!(t.habit_id, Some(9));

        t.start(T0);
        let r = t.skip(T0 + 12 * MIN, &s).unwrap();
        assert_eq!(r.duration_ms, 12 * MIN);
        assert_eq!(r.habit_id, Some(9));
        assert_eq!(t.phase, Phase::ShortBreak);
        assert_eq!(t.focus_count, 0, "foco pulado não conta para a pausa longa");
    }

    #[test]
    fn cronometro() {
        let s = Settings::default();
        let mut t = Timer::default();
        t.set_mode(Mode::Stopwatch);
        t.start(T0);
        assert!(t.tick(T0 + 90 * MIN, &s).is_none(), "cronômetro não termina sozinho");
        assert_eq!(t.display_ms(T0 + 90 * MIN, &s), 90 * MIN);
        let r = t.stop(T0 + 90 * MIN).unwrap();
        assert_eq!(r.mode, Mode::Stopwatch);
        assert_eq!(r.duration_ms, 90 * MIN);
    }

    #[test]
    fn credito_no_habito() {
        let mut h = Habit::new("Estudar", T0);
        assert_eq!(focus_credit(&h, 25 * MIN), Credit::None, "hábito de parar não recebe crédito");
        h.kind = HabitKind::Build;
        assert_eq!(focus_credit(&h, 25 * MIN), Credit::Check);
        h.daily_target = 50.0;
        h.target_unit = "Minutos".into();
        assert_eq!(focus_credit(&h, 25 * MIN), Credit::AddProgress(25.0));
        h.target_unit = "h".into();
        assert_eq!(focus_credit(&h, 30 * MIN), Credit::AddProgress(0.5));
        h.target_unit = "páginas".into();
        assert_eq!(focus_credit(&h, 25 * MIN), Credit::None);
    }

    #[test]
    fn resumo() {
        let today = local_date(T0 + 12 * 60 * MIN);
        let rec = |start: i64, minutes: i64| FocusRecord {
            id: 0,
            habit_id: None,
            started_at: start,
            ended_at: start + minutes * MIN,
            duration_ms: minutes * MIN,
            mode: Mode::Pomodoro,
        };
        let noon = T0 + 12 * 60 * MIN;
        let records = vec![rec(noon, 25), rec(noon + 30 * MIN, 25), rec(noon - 2 * MS_PER_DAY, 50)];
        let s = summarize(&records, today);
        assert_eq!(s.today_ms, 50 * MIN);
        assert_eq!(s.today_sessions, 2);
        assert_eq!(s.week.len(), 7);
        assert_eq!(s.week.last().unwrap().0, today);
        assert_eq!(s.week_ms, 100 * MIN);
    }

    #[test]
    fn configuracoes_persistem() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(Settings::load(&db), Settings::default());
        let custom = Settings { focus_min: 50, auto_start_focus: true, ..Settings::default() };
        custom.save(&db).unwrap();
        assert_eq!(Settings::load(&db), custom);
    }
}

#[cfg(test)]
mod comparison_tests {
    use super::*;
    use chrono::TimeZone;

    fn session(y: i32, m: u32, d: u32, minutes: i64) -> FocusRecord {
        let start = chrono::Local
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap();
        FocusRecord {
            id: 0,
            habit_id: None,
            started_at: start,
            ended_at: start + minutes * MS_PER_MINUTE,
            duration_ms: minutes * MS_PER_MINUTE,
            mode: Mode::Pomodoro,
        }
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, d).unwrap()
    }

    #[test]
    fn compara_ate_o_mesmo_ponto_do_periodo() {
        // Semana anterior: seg 6 e ter 7 (100 min) + qui 9 (fora da janela seg–qua).
        // Semana atual até quarta 15: 50 + 50 + 20 = 120 min → +20%.
        let records = vec![
            session(2025, 1, 6, 50),
            session(2025, 1, 7, 50),
            session(2025, 1, 9, 60),
            session(2025, 1, 13, 50),
            session(2025, 1, 14, 50),
            session(2025, 1, 15, 20),
        ];
        let c = compare_with_previous(&records, Period::Week, day(15), day(15));
        assert!(c.partial);
        assert_eq!(c.current.ms, 120 * MS_PER_MINUTE);
        assert_eq!(c.previous.ms, 100 * MS_PER_MINUTE);
        assert!((c.time_change().unwrap() - 0.2).abs() < 1e-9);
        assert!((c.pomodoro_change().unwrap() - 0.5).abs() < 1e-9); // 3 × 2 sessões
        assert_eq!(
            comparison_text(&c, Period::Week),
            "+20% de foco em relação à semana anterior (até o mesmo dia)"
        );

        // Semana passada inteira (já encerrada) contra a retrasada, que não teve foco.
        let past = compare_with_previous(&records, Period::Week, day(8), day(15));
        assert!(!past.partial);
        assert_eq!(past.current.ms, 160 * MS_PER_MINUTE);
        assert_eq!(past.time_change(), None);
        assert_eq!(comparison_text(&past, Period::Week), "Sem foco no período anterior para comparar.");
    }

    #[test]
    fn queda_e_sem_dados() {
        let records = vec![session(2024, 12, 10, 100), session(2025, 1, 10, 75)];
        // No último dia do período a comparação já é com o mês anterior inteiro.
        assert!(!compare_with_previous(&records, Period::Month, day(20), day(31)).partial);
        let full = compare_with_previous(&records, Period::Month, day(20), NaiveDate::from_ymd_opt(2025, 2, 5).unwrap());
        assert!(!full.partial);
        assert_eq!(comparison_text(&full, Period::Month), "−25% de foco em relação ao mês anterior");
        let empty = compare_with_previous(&[], Period::Year, day(1), day(1));
        assert_eq!(comparison_text(&empty, Period::Year), "Sem foco neste período nem no anterior.");
    }
}
