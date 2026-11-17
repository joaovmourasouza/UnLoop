//! Metas em cascata, como no second brain: Ano → Semestre → Trimestre → Mês → Semana.
//!
//! Cada meta pertence a um período (ex.: "2026-T4") e pode ter uma meta-mãe de um nível
//! acima cujo período a contenha. Tarefas e hábitos se ligam às metas.
//!
//! Regras:
//! - A semana ISO pertence ao mês (e trimestre…) da sua quinta-feira: W44 (26/10–01/11) é
//!   de outubro, W45 (02–08/11) é de novembro.
//! - Progresso: média do progresso das filhas não riscadas (sobe pela cascata: a semana
//!   em 50% puxa o mês, o trimestre…); sem filhas, tarefas concluídas / tarefas ligadas;
//!   sem nada, 0 ou 100%. Meta concluída conta 100%.
//! - Meta riscada (não rodou, remanejada) não entra na conta da mãe.

use chrono::{Datelike, NaiveDate, Weekday};
use rusqlite::{params, OptionalExtension, Result, Row};

use crate::db::Db;
use crate::model::Task;

/// Metas (tabelas `objective` e `objective_habit`). Aplicado por [`Db::ensure_objectives`]
/// sempre que falta, sem depender de `PRAGMA user_version` (que outras migrações numeram).
pub const OBJECTIVES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS objective (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id   INTEGER REFERENCES objective(id) ON DELETE CASCADE,
    level       INTEGER NOT NULL,
    period      TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    area        TEXT    NOT NULL DEFAULT '',
    status      INTEGER NOT NULL DEFAULT 0,
    status_note TEXT    NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    closed_at   INTEGER
);
CREATE INDEX IF NOT EXISTS ix_objective_period ON objective (level, period);
CREATE INDEX IF NOT EXISTS ix_objective_parent ON objective (parent_id);
CREATE TABLE IF NOT EXISTS objective_habit (
    objective_id INTEGER NOT NULL REFERENCES objective(id) ON DELETE CASCADE,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    PRIMARY KEY (objective_id, habit_id)
);
"#;

// ------------------------------------------------------------------ Níveis

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Year,
    Semester,
    Quarter,
    Month,
    Week,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Year, Level::Semester, Level::Quarter, Level::Month, Level::Week];

    pub fn code(self) -> i64 {
        self as i64
    }

    pub fn from_code(code: i64) -> Level {
        Level::ALL.get(code.clamp(0, 4) as usize).copied().unwrap_or(Level::Year)
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Year => "Ano",
            Level::Semester => "Semestre",
            Level::Quarter => "Trimestre",
            Level::Month => "Mês",
            Level::Week => "Semana",
        }
    }

    /// Chave usada na CLI e no backup.
    pub fn key(self) -> &'static str {
        match self {
            Level::Year => "ano",
            Level::Semester => "semestre",
            Level::Quarter => "trimestre",
            Level::Month => "mes",
            Level::Week => "semana",
        }
    }

    pub fn parse(text: &str) -> Option<Level> {
        match fold(text).as_str() {
            "ano" | "anual" | "year" | "y" => Some(Level::Year),
            "semestre" | "semestral" | "semester" | "s" => Some(Level::Semester),
            "trimestre" | "trimestral" | "tri" | "quarter" | "t" | "q" => Some(Level::Quarter),
            "mes" | "mensal" | "month" | "m" => Some(Level::Month),
            "semana" | "semanal" | "week" | "w" => Some(Level::Week),
            _ => None,
        }
    }

    /// Nível logo abaixo (Ano → Semestre …).
    pub fn child(self) -> Option<Level> {
        Level::ALL.get(self as usize + 1).copied()
    }

    pub fn parent(self) -> Option<Level> {
        (self as usize).checked_sub(1).and_then(|i| Level::ALL.get(i).copied())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Open,
    Done,
    /// Não rodou / remanejada / saiu do planejamento (no second brain, fica riscada).
    Dropped,
}

impl Status {
    pub fn code(self) -> i64 {
        match self {
            Status::Open => 0,
            Status::Done => 1,
            Status::Dropped => 2,
        }
    }

    pub fn from_code(code: i64) -> Status {
        match code {
            1 => Status::Done,
            2 => Status::Dropped,
            _ => Status::Open,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Done => "done",
            Status::Dropped => "dropped",
        }
    }

    pub fn from_key(key: &str) -> Status {
        match key {
            "done" => Status::Done,
            "dropped" => Status::Dropped,
            _ => Status::Open,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Open => "Em andamento",
            Status::Done => "Concluída",
            Status::Dropped => "Riscada",
        }
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Status::Open => "🎯",
            Status::Done => "✅",
            Status::Dropped => "➖",
        }
    }
}

// ---------------------------------------------------------------- Períodos

const MONTHS: [&str; 12] = [
    "Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro", "Novembro",
    "Dezembro",
];
const MONTHS_SHORT: [&str; 12] = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];

/// Minúsculas, sem acentos e sem espaços nas pontas.
fn fold(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Chave do período de `level` que contém `day`: "2026", "2026-S2", "2026-T4", "2026-11", "2026-W47".
pub fn period_key(level: Level, day: NaiveDate) -> String {
    let (y, m) = (day.year(), day.month());
    match level {
        Level::Year => format!("{y}"),
        Level::Semester => format!("{y}-S{}", if m <= 6 { 1 } else { 2 }),
        Level::Quarter => format!("{y}-T{}", (m - 1) / 3 + 1),
        Level::Month => format!("{y}-{m:02}"),
        Level::Week => {
            let w = day.iso_week();
            format!("{}-W{:02}", w.year(), w.week())
        }
    }
}

fn month_last_day(y: i32, m: u32) -> Option<NaiveDate> {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)?.pred_opt()
}

/// Primeiro e último dia do período (`None` se a chave for inválida).
pub fn period_range(level: Level, key: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (year, rest) = match key.split_once('-') {
        Some((y, r)) => (y.parse::<i32>().ok()?, Some(r)),
        None => (key.parse::<i32>().ok()?, None),
    };
    if !(1900..=2999).contains(&year) {
        return None;
    }
    let months = |first: u32, last: u32| Some((NaiveDate::from_ymd_opt(year, first, 1)?, month_last_day(year, last)?));
    match (level, rest) {
        (Level::Year, None) => months(1, 12),
        (Level::Semester, Some(r)) => match r {
            "S1" => months(1, 6),
            "S2" => months(7, 12),
            _ => None,
        },
        (Level::Quarter, Some(r)) => {
            let q: u32 = r.strip_prefix('T')?.parse().ok()?;
            (1..=4).contains(&q).then_some(())?;
            months(3 * q - 2, 3 * q)
        }
        (Level::Month, Some(r)) => {
            let m: u32 = r.parse().ok()?;
            (r.len() == 2 && (1..=12).contains(&m)).then_some(())?;
            months(m, m)
        }
        (Level::Week, Some(r)) => {
            let w: u32 = r.strip_prefix('W')?.parse().ok()?;
            let start = NaiveDate::from_isoywd_opt(year, w, Weekday::Mon)?;
            Some((start, NaiveDate::from_isoywd_opt(year, w, Weekday::Sun)?))
        }
        _ => None,
    }
}

/// Dia que representa o período para achar os períodos de cima (semana: a quinta-feira).
pub fn anchor(level: Level, key: &str) -> Option<NaiveDate> {
    let (start, _) = period_range(level, key)?;
    Some(if level == Level::Week { start + chrono::Days::new(3) } else { start })
}

/// O período de `parent_level` que contém o período filho.
pub fn parent_period(parent_level: Level, level: Level, key: &str) -> Option<String> {
    anchor(level, key).map(|d| period_key(parent_level, d))
}

/// O período `(level, key)` está dentro de `(parent_level, parent_key)`?
pub fn contains(parent_level: Level, parent_key: &str, level: Level, key: &str) -> bool {
    parent_level < level && parent_period(parent_level, level, key).as_deref() == Some(parent_key)
}

/// Rótulo curto: "2026", "S2", "T4", "Nov", "W47".
pub fn short_label(level: Level, key: &str) -> String {
    match (level, key.split_once('-')) {
        (Level::Month, Some((_, m))) => m
            .parse::<usize>()
            .ok()
            .and_then(|m| MONTHS_SHORT.get(m.wrapping_sub(1)))
            .map(|s| {
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            })
            .unwrap_or_else(|| key.to_string()),
        (_, Some((_, rest))) => rest.to_string(),
        _ => key.to_string(),
    }
}

/// Rótulo completo: "2026", "S2 · jul–dez 2026", "T4 · out–dez 2026", "Novembro 2026", "W47 · 16/11–22/11".
pub fn period_label(level: Level, key: &str) -> String {
    let Some((start, end)) = period_range(level, key) else { return key.to_string() };
    let short = |d: NaiveDate| MONTHS_SHORT[d.month0() as usize];
    match level {
        Level::Year => key.to_string(),
        Level::Semester | Level::Quarter => {
            format!("{} · {}–{} {}", short_label(level, key), short(start), short(end), start.year())
        }
        Level::Month => format!("{} {}", MONTHS[start.month0() as usize], start.year()),
        Level::Week => format!("{} · {}–{}", short_label(level, key), start.format("%d/%m"), end.format("%d/%m")),
    }
}

/// Lê um período digitado. Aceita "2026", "S2", "2026-S2", "T4", "Q4", "2026-T4", "nov",
/// "novembro 2026", "11/2026", "2026-11", "W47", "2026-W47", "atual" (com `hint`) e datas
/// ("hoje", "16/11") quando `hint` diz o nível.
pub fn parse_period(text: &str, hint: Option<Level>, today: NaiveDate) -> Option<(Level, String)> {
    let t = fold(text).replace(' ', "");
    let year = today.year();
    let ok = |level: Level, key: String| period_range(level, &key).map(|_| (level, key));
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());

    if t.is_empty() || t == "atual" || t == "current" {
        let level = hint?;
        return Some((level, period_key(level, today)));
    }
    if t.len() == 4 && digits(&t) {
        return ok(Level::Year, t);
    }
    // Com ano na frente: "2026-s2", "2026t4", "2026-w47", "2026-11".
    let year_prefix = t.get(..4).filter(|p| t.len() > 4 && digits(p));
    let (y, rest) = match year_prefix {
        Some(p) => (p.parse::<i32>().ok()?, t[4..].trim_start_matches('-').to_string()),
        None => (year, t.clone()),
    };
    if let Some(n) = rest.strip_prefix('s').filter(|n| digits(n)) {
        return ok(Level::Semester, format!("{y}-S{n}"));
    }
    if let Some(n) = rest.strip_prefix('t').or_else(|| rest.strip_prefix('q')).filter(|n| digits(n)) {
        return ok(Level::Quarter, format!("{y}-T{n}"));
    }
    if let Some(n) = rest.strip_prefix('w').filter(|n| digits(n)) {
        // Sem ano explícito, a semana é do ano ISO de hoje.
        let wy = if year_prefix.is_some() { y } else { today.iso_week().year() };
        return ok(Level::Week, format!("{wy}-W{:02}", n.parse::<u32>().ok()?));
    }
    if year_prefix.is_some() && digits(&rest) && rest.len() <= 2 {
        return ok(Level::Month, format!("{y}-{:02}", rest.parse::<u32>().ok()?));
    }
    // "11/2026"
    if let Some((m, yy)) = t.split_once('/') {
        if digits(m) && yy.len() == 4 && digits(yy) {
            return ok(Level::Month, format!("{yy}-{:02}", m.parse::<u32>().ok()?));
        }
    }
    // Nome do mês, com ano opcional: "nov", "novembro2026", "nov/2026".
    let letters: String = t.chars().take_while(|c| c.is_alphabetic()).collect();
    if letters.len() >= 3 {
        let tail = t[letters.len()..].trim_start_matches('/');
        let my = if tail.is_empty() { Some(year) } else if digits(tail) && tail.len() == 4 { tail.parse().ok() } else { None };
        if let (Some(i), Some(my)) =
            (MONTHS.iter().position(|m| fold(m).starts_with(&letters) || fold(m) == letters), my)
        {
            let first3: String = letters.chars().take(3).collect();
            if MONTHS_SHORT[i].starts_with(&first3) {
                return ok(Level::Month, format!("{my}-{:02}", i + 1));
            }
        }
    }
    // Data ("hoje", "16/11", "2026-11-16") com o nível vindo de `hint`.
    let level = hint?;
    let (ms, _) = crate::tasks::parse_due(text, today)?;
    Some((level, period_key(level, crate::checkin::local_date(ms))))
}

/// Períodos de `level` (em ordem) cuja âncora cai entre `from` e `to`.
pub fn periods_between(level: Level, from: NaiveDate, to: NaiveDate) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut day = from;
    while day <= to {
        let key = period_key(level, day);
        if anchor(level, &key).is_some_and(|a| a >= from && a <= to) && out.last() != Some(&key) {
            out.push(key);
        }
        match day.succ_opt() {
            Some(d) => day = d,
            None => break,
        }
    }
    out
}

/// Os períodos de hoje, do ano até a semana.
pub fn current_chain(today: NaiveDate) -> Vec<(Level, String)> {
    // A semana é a de hoje; os níveis de cima são os de hoje (não os da quinta da semana).
    Level::ALL.iter().map(|&l| (l, period_key(l, today))).collect()
}

// ------------------------------------------------------------------- Metas

#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub level: Level,
    pub period: String,
    pub title: String,
    /// Markdown.
    pub description: String,
    /// Área livre (ex.: "Estudo", "Financeiro").
    pub area: String,
    pub status: Status,
    /// Motivo de riscar ou de concluir (ex.: "remanejado pra W48").
    pub status_note: String,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: Option<i64>,
    /// Hábitos ligados.
    pub habit_ids: Vec<i64>,
}

impl Objective {
    pub fn new(level: Level, period: &str, title: &str) -> Self {
        Objective {
            id: 0,
            parent_id: None,
            level,
            period: period.to_string(),
            title: title.trim().to_string(),
            description: String::new(),
            area: String::new(),
            status: Status::Open,
            status_note: String::new(),
            position: 0,
            created_at: 0,
            updated_at: 0,
            closed_at: None,
            habit_ids: Vec::new(),
        }
    }

    pub fn period_label(&self) -> String {
        period_label(self.level, &self.period)
    }

    pub fn range(&self) -> Option<(NaiveDate, NaiveDate)> {
        period_range(self.level, &self.period)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Filhas (ou tarefas) concluídas.
    pub done: usize,
    /// Filhas não riscadas (ou tarefas ligadas).
    pub total: usize,
    /// 0.0–1.0, já com o andamento parcial das filhas.
    pub value: f64,
}

impl Progress {
    pub fn fraction(self) -> f64 {
        self.value.clamp(0.0, 1.0)
    }

    pub fn percent(self) -> u32 {
        (self.fraction() * 100.0).round() as u32
    }
}

/// Filhas diretas, ordenadas por período e posição.
pub fn children<'a>(all: &'a [Objective], id: i64) -> Vec<&'a Objective> {
    let mut out: Vec<&Objective> = all.iter().filter(|o| o.parent_id == Some(id)).collect();
    out.sort_by(|a, b| (a.level, &a.period, a.position, a.id).cmp(&(b.level, &b.period, b.position, b.id)));
    out
}

/// Todos os descendentes (filhas, netas…).
pub fn descendants(all: &[Objective], id: i64) -> Vec<&Objective> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        for c in all.iter().filter(|o| o.parent_id == Some(current)) {
            out.push(c);
            stack.push(c.id);
        }
    }
    out
}

/// Caminho da raiz até a meta (inclusive).
pub fn ancestry<'a>(all: &'a [Objective], id: i64) -> Vec<&'a Objective> {
    let mut out = Vec::new();
    let mut current = all.iter().find(|o| o.id == id);
    while let Some(o) = current {
        if out.iter().any(|x: &&Objective| x.id == o.id) {
            break; // ciclo (não deveria acontecer)
        }
        out.push(o);
        current = o.parent_id.and_then(|p| all.iter().find(|x| x.id == p));
    }
    out.reverse();
    out
}

pub fn progress(o: &Objective, all: &[Objective], tasks: &[Task]) -> Progress {
    progress_inner(o, all, tasks, 0)
}

fn progress_inner(o: &Objective, all: &[Objective], tasks: &[Task], depth: usize) -> Progress {
    let kids: Vec<&Objective> = children(all, o.id).into_iter().filter(|c| c.status != Status::Dropped).collect();
    let p = if !kids.is_empty() && depth < 8 {
        let sum: f64 = kids.iter().map(|c| progress_inner(c, all, tasks, depth + 1).fraction()).sum();
        Progress {
            done: kids.iter().filter(|c| c.status == Status::Done).count(),
            total: kids.len(),
            value: sum / kids.len() as f64,
        }
    } else {
        let linked: Vec<&Task> = tasks.iter().filter(|t| t.objective_id == Some(o.id)).collect();
        let (done, total) = if linked.is_empty() {
            (usize::from(o.status == Status::Done), 1)
        } else {
            (linked.iter().filter(|t| t.is_done()).count(), linked.len())
        };
        Progress { done, total, value: done as f64 / total as f64 }
    };
    if o.status == Status::Done {
        Progress { done: p.total, total: p.total, value: 1.0 }
    } else {
        p
    }
}

/// Confere se `parent` pode ser a mãe de uma meta `(level, period)`.
pub fn check_parent(level: Level, period: &str, parent: &Objective) -> std::result::Result<(), String> {
    if parent.level >= level {
        return Err(format!(
            "a meta-mãe precisa ser de um nível acima ({} não fica dentro de {})",
            level.label().to_lowercase(),
            parent.level.label().to_lowercase()
        ));
    }
    if !contains(parent.level, &parent.period, level, period) {
        return Err(format!(
            "o período {} não fica dentro de {} (período da meta-mãe)",
            period_label(level, period),
            parent.period_label()
        ));
    }
    Ok(())
}

// --------------------------------------------------------------------- Banco

fn objective_from_row(r: &Row) -> Result<Objective> {
    Ok(Objective {
        id: r.get("id")?,
        parent_id: r.get("parent_id")?,
        level: Level::from_code(r.get("level")?),
        period: r.get("period")?,
        title: r.get("title")?,
        description: r.get("description")?,
        area: r.get("area")?,
        status: Status::from_code(r.get("status")?),
        status_note: r.get("status_note")?,
        position: r.get("position")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        closed_at: r.get("closed_at")?,
        habit_ids: Vec::new(),
    })
}

impl Db {
    /// Cria as tabelas de metas e a coluna `task.objective_id` se faltarem. Idempotente.
    pub fn ensure_objectives(&self) -> Result<()> {
        self.transaction(|| {
            self.conn.execute_batch(OBJECTIVES_SCHEMA)?;
            let has_column: bool = self.conn.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('task') WHERE name = 'objective_id'",
                [],
                |r| r.get(0),
            )?;
            if !has_column {
                self.conn.execute_batch(
                    "ALTER TABLE task ADD COLUMN objective_id INTEGER REFERENCES objective(id) ON DELETE SET NULL;",
                )?;
            }
            Ok(())
        })
    }

    /// Todas as metas (com os hábitos ligados), do ano para a semana.
    pub fn objectives(&self) -> Result<Vec<Objective>> {
        let mut st = self.conn.prepare("SELECT * FROM objective ORDER BY level, period, position, id")?;
        let mut all: Vec<Objective> = st.query_map([], objective_from_row)?.collect::<Result<_>>()?;
        let mut st = self.conn.prepare("SELECT objective_id, habit_id FROM objective_habit ORDER BY habit_id")?;
        let links: Vec<(i64, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_>>()?;
        for o in &mut all {
            o.habit_ids = links.iter().filter(|(oid, _)| *oid == o.id).map(|(_, h)| *h).collect();
        }
        Ok(all)
    }

    pub fn objective(&self, id: i64) -> Result<Option<Objective>> {
        let Some(mut o) =
            self.conn.query_row("SELECT * FROM objective WHERE id = ?1", [id], objective_from_row).optional()?
        else {
            return Ok(None);
        };
        let mut st = self.conn.prepare("SELECT habit_id FROM objective_habit WHERE objective_id = ?1 ORDER BY habit_id")?;
        o.habit_ids = st.query_map([id], |r| r.get(0))?.collect::<Result<_>>()?;
        Ok(Some(o))
    }

    fn set_objective_habits(&self, id: i64, habit_ids: &[i64]) -> Result<()> {
        self.conn.execute("DELETE FROM objective_habit WHERE objective_id = ?1", [id])?;
        for h in habit_ids {
            self.conn.execute(
                "INSERT OR IGNORE INTO objective_habit (objective_id, habit_id) VALUES (?1, ?2)",
                params![id, h],
            )?;
        }
        Ok(())
    }

    pub fn insert_objective(&self, o: &Objective) -> Result<i64> {
        self.transaction(|| {
            let now = crate::now_ms();
            self.conn.execute(
                "INSERT INTO objective (parent_id, level, period, title, description, area, status, status_note, \
                 position, created_at, updated_at, closed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, \
                 (SELECT COALESCE(MAX(position), -1) + 1 FROM objective WHERE level = ?2 AND period = ?3), ?9, ?10, ?11)",
                params![
                    o.parent_id,
                    o.level.code(),
                    o.period,
                    o.title.trim(),
                    o.description.trim(),
                    o.area.trim(),
                    o.status.code(),
                    o.status_note.trim(),
                    if o.created_at == 0 { now } else { o.created_at },
                    now,
                    o.closed_at,
                ],
            )?;
            let id = self.conn.last_insert_rowid();
            self.set_objective_habits(id, &o.habit_ids)?;
            Ok(id)
        })
    }

    pub fn update_objective(&self, o: &Objective) -> Result<()> {
        self.transaction(|| {
            self.conn.execute(
                "UPDATE objective SET parent_id = ?1, level = ?2, period = ?3, title = ?4, description = ?5, \
                 area = ?6, status = ?7, status_note = ?8, closed_at = ?9, updated_at = ?10 WHERE id = ?11",
                params![
                    o.parent_id,
                    o.level.code(),
                    o.period,
                    o.title.trim(),
                    o.description.trim(),
                    o.area.trim(),
                    o.status.code(),
                    o.status_note.trim(),
                    o.closed_at,
                    crate::now_ms(),
                    o.id,
                ],
            )?;
            self.set_objective_habits(o.id, &o.habit_ids)
        })
    }

    /// Conclui, risca ou reabre. `note`: motivo (vazio mantém o atual ao reabrir? não: limpa).
    pub fn set_objective_status(&self, id: i64, status: Status, note: &str) -> Result<()> {
        let now = crate::now_ms();
        self.conn.execute(
            "UPDATE objective SET status = ?1, status_note = ?2, closed_at = ?3, updated_at = ?4 WHERE id = ?5",
            params![status.code(), note.trim(), (status != Status::Open).then_some(now), now, id],
        )?;
        Ok(())
    }

    /// Apaga a meta e as filhas; as tarefas ligadas ficam sem meta.
    pub fn delete_objective(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM objective WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_all_objectives(&self) -> Result<()> {
        self.conn.execute("DELETE FROM objective", [])?;
        Ok(())
    }

    pub fn set_task_objective(&self, task_id: i64, objective_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE task SET objective_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![objective_id, crate::now_ms(), task_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn chaves_e_intervalos() {
        let day = d(2026, 11, 16);
        assert_eq!(period_key(Level::Year, day), "2026");
        assert_eq!(period_key(Level::Semester, day), "2026-S2");
        assert_eq!(period_key(Level::Quarter, day), "2026-T4");
        assert_eq!(period_key(Level::Month, day), "2026-11");
        assert_eq!(period_key(Level::Week, day), "2026-W47");
        assert_eq!(period_range(Level::Week, "2026-W47"), Some((d(2026, 11, 16), d(2026, 11, 22))));
        assert_eq!(period_range(Level::Quarter, "2026-T4"), Some((d(2026, 10, 1), d(2026, 12, 31))));
        assert_eq!(period_range(Level::Month, "2026-02"), Some((d(2026, 2, 1), d(2026, 2, 28))));
        assert_eq!(period_range(Level::Semester, "2026-S3"), None);
        assert_eq!(period_range(Level::Month, "2026-13"), None);
        assert_eq!(period_range(Level::Year, "2026-T4"), None);
        assert_eq!(period_label(Level::Semester, "2026-S2"), "S2 · jul–dez 2026");
        assert_eq!(period_label(Level::Quarter, "2026-T4"), "T4 · out–dez 2026");
        assert_eq!(period_label(Level::Month, "2026-11"), "Novembro 2026");
        assert_eq!(period_label(Level::Week, "2026-W47"), "W47 · 16/11–22/11");
        assert_eq!(short_label(Level::Month, "2026-11"), "Nov");
    }

    #[test]
    fn semana_pertence_ao_mes_da_quinta() {
        // W44 = 26/10–01/11 (quinta 29/10): outubro. W45 = 02–08/11: novembro.
        assert!(contains(Level::Month, "2026-10", Level::Week, "2026-W44"));
        assert!(!contains(Level::Month, "2026-11", Level::Week, "2026-W44"));
        assert!(contains(Level::Month, "2026-11", Level::Week, "2026-W45"));
        assert!(contains(Level::Quarter, "2026-T4", Level::Month, "2026-11"));
        assert!(contains(Level::Year, "2026", Level::Week, "2026-W47"));
        assert!(!contains(Level::Month, "2026-11", Level::Month, "2026-11"), "mesmo nível não é mãe");
        // Semana 1 de 2027 começa em 04/01/2027; a W53 de 2026 (28/12–03/01) é de dezembro.
        assert!(contains(Level::Month, "2026-12", Level::Week, "2026-W53"));
        assert_eq!(periods_between(Level::Week, d(2026, 11, 1), d(2026, 11, 30)).len(), 4);
        assert_eq!(
            periods_between(Level::Month, d(2026, 10, 1), d(2026, 12, 31)),
            vec!["2026-10", "2026-11", "2026-12"]
        );
    }

    #[test]
    fn ler_periodo_digitado() {
        let today = d(2026, 11, 16);
        let p = |s: &str| parse_period(s, None, today);
        assert_eq!(p("2026"), Some((Level::Year, "2026".into())));
        assert_eq!(p("S2"), Some((Level::Semester, "2026-S2".into())));
        assert_eq!(p("2027-s1"), Some((Level::Semester, "2027-S1".into())));
        assert_eq!(p("T4"), Some((Level::Quarter, "2026-T4".into())));
        assert_eq!(p("q1"), Some((Level::Quarter, "2026-T1".into())));
        assert_eq!(p("2026-11"), Some((Level::Month, "2026-11".into())));
        assert_eq!(p("11/2026"), Some((Level::Month, "2026-11".into())));
        assert_eq!(p("nov"), Some((Level::Month, "2026-11".into())));
        assert_eq!(p("Novembro 2026"), Some((Level::Month, "2026-11".into())));
        assert_eq!(p("dez/2026"), Some((Level::Month, "2026-12".into())));
        assert_eq!(p("março"), Some((Level::Month, "2026-03".into())));
        assert_eq!(p("W47"), Some((Level::Week, "2026-W47".into())));
        assert_eq!(p("2026-w5"), Some((Level::Week, "2026-W05".into())));
        assert_eq!(p("T5"), None);
        assert_eq!(p("hoje"), None, "data sem nível é ambígua");
        assert_eq!(parse_period("hoje", Some(Level::Week), today), Some((Level::Week, "2026-W47".into())));
        assert_eq!(parse_period("atual", Some(Level::Quarter), today), Some((Level::Quarter, "2026-T4".into())));
        assert_eq!(parse_period("05/12", Some(Level::Week), today), Some((Level::Week, "2026-W49".into())));
    }

    #[test]
    fn progresso_e_mae() {
        let mut year = Objective::new(Level::Year, "2026", "Curso até ~#085");
        year.id = 1;
        let mut t4 = Objective::new(Level::Quarter, "2026-T4", "Curso #052 → ~#085");
        t4.id = 2;
        t4.parent_id = Some(1);
        let mut nov = Objective::new(Level::Month, "2026-11", "Fechar o Módulo 2");
        nov.id = 3;
        nov.parent_id = Some(2);
        nov.status = Status::Done;
        let mut dec = Objective::new(Level::Month, "2026-12", "Começar o Módulo 3");
        dec.id = 4;
        dec.parent_id = Some(2);
        let mut oct = Objective::new(Level::Month, "2026-10", "Algo que saiu do plano");
        oct.id = 5;
        oct.parent_id = Some(2);
        oct.status = Status::Dropped;
        let all = vec![year.clone(), t4.clone(), nov.clone(), dec.clone(), oct];

        let t4p = progress(&t4, &all, &[]);
        assert_eq!((t4p.done, t4p.total, t4p.percent()), (1, 2, 50), "riscada não conta");
        assert_eq!(progress(&year, &all, &[]).percent(), 50, "o ano herda o andamento do trimestre");
        let mut task = Task::new("Aula #070");
        task.objective_id = Some(4);
        let mut done_task = Task::new("Aula #069");
        done_task.objective_id = Some(4);
        done_task.done_at = Some(1);
        let tasks = [task, done_task];
        let decp = progress(&dec, &all, &tasks);
        assert_eq!((decp.done, decp.total, decp.percent()), (1, 2, 50));
        // Novembro concluído (100%) + dezembro pela metade (50%) = trimestre em 75%.
        assert_eq!(progress(&t4, &all, &tasks).percent(), 75);
        assert_eq!(progress(&year, &all, &tasks).percent(), 75);
        assert_eq!(progress(&nov, &all, &[]).percent(), 100);

        assert_eq!(ancestry(&all, 3).iter().map(|o| o.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(descendants(&all, 1).len(), 4);
        assert!(check_parent(Level::Month, "2026-11", &t4).is_ok());
        assert!(check_parent(Level::Month, "2027-01", &t4).is_err());
        assert!(check_parent(Level::Quarter, "2026-T4", &nov).is_err());
    }

    #[test]
    fn esquema_idempotente() {
        let db = Db::open_in_memory().unwrap();
        db.ensure_objectives().unwrap();
        db.ensure_objectives().unwrap();
        assert!(db.objectives().unwrap().is_empty());
    }

    #[test]
    fn banco() {
        let db = Db::open_in_memory().unwrap();
        let mut h = crate::model::Habit::new("Acordar cedo", 0);
        h.kind = crate::model::HabitKind::Build;
        let hid = db.insert_habit(&h, false).unwrap();

        let year = db.insert_objective(&Objective::new(Level::Year, "2026", "Curso")).unwrap();
        let mut month = Objective::new(Level::Month, "2026-11", "Módulo 2");
        month.parent_id = Some(year);
        month.habit_ids = vec![hid];
        let mid = db.insert_objective(&month).unwrap();
        assert_eq!(db.objective(mid).unwrap().unwrap().habit_ids, vec![hid]);

        let mut task = Task::new("Aula #065");
        task.objective_id = Some(mid);
        let tid = db.insert_task(&task).unwrap();
        assert_eq!(db.task(tid).unwrap().unwrap().objective_id, Some(mid));

        db.set_objective_status(mid, Status::Dropped, "remanejado pra dezembro").unwrap();
        let o = db.objective(mid).unwrap().unwrap();
        assert_eq!(o.status, Status::Dropped);
        assert_eq!(o.status_note, "remanejado pra dezembro");
        assert!(o.closed_at.is_some());

        // Apagar a mãe apaga a filha e solta a tarefa.
        db.delete_objective(year).unwrap();
        assert!(db.objectives().unwrap().is_empty());
        assert_eq!(db.task(tid).unwrap().unwrap().objective_id, None);
    }
}
