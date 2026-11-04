//! Persistência SQLite. Migrações controladas por `PRAGMA user_version`.
//! O DDL espelha docs/schema.sql.

use std::cell::Cell;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Result, Row};

use crate::calc::HabitBundle;
use crate::model::*;
use crate::presets::{goal_presets, DEFAULT_AFFIRMATIONS};

pub const SCHEMA_VERSION: i64 = 3;

/// Tabelas de texto simples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTable {
    Reason,
    Affirmation,
}

impl TextTable {
    fn table(self) -> &'static str {
        match self {
            TextTable::Reason => "reason",
            TextTable::Affirmation => "affirmation",
        }
    }
}

pub struct Db {
    conn: Connection,
    tx_depth: Cell<u32>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Db { conn, tx_depth: Cell::new(0) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let current: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }
        self.transaction(|| {
            if current < 1 {
                self.conn.execute_batch(SCHEMA_V1)?;
            }
            if current < 2 {
                self.conn.execute_batch(MIGRATION_V2)?;
            }
            if current < 3 {
                self.conn.execute_batch(MIGRATION_V3)?;
            }
            // Próximas versões: if current < 4 { self.conn.execute_batch(MIGRATION_V4)?; }
            self.conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        })
    }

    /// Executa `f` numa transação (aninhadas reaproveitam a externa).
    pub fn transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.tx_depth.get() > 0 {
            return f();
        }
        self.conn.execute_batch("BEGIN")?;
        self.tx_depth.set(1);
        let result = f();
        self.tx_depth.set(0);
        match result {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ---------------------------------------------------------------- Hábitos

    pub fn habits(&self) -> Result<Vec<Habit>> {
        let mut st = self.conn.prepare("SELECT * FROM habit ORDER BY position, id")?;
        let rows = st.query_map([], habit_from_row)?;
        rows.collect()
    }

    pub fn habit(&self, id: i64) -> Result<Option<Habit>> {
        self.conn
            .query_row("SELECT * FROM habit WHERE id = ?1", [id], habit_from_row)
            .optional()
    }

    /// Cria o hábito. Com `seed_defaults`, cria as metas padrão e as afirmações iniciais.
    pub fn insert_habit(&self, h: &Habit, seed_defaults: bool) -> Result<i64> {
        self.transaction(|| {
            let created = if h.created_at == 0 { crate::now_ms() } else { h.created_at };
            self.conn.execute(
                "INSERT INTO habit (name, icon, color, quit_at, daily_money, daily_minutes, \
                 daily_frequency, habit_since_ms, preferred_metric, counter_type, counter_style, \
                 position, created_at, kind, schedule_days, daily_target, target_unit) VALUES (?1, ?2, ?3, \
                 ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, (SELECT COALESCE(MAX(position), -1) + 1 FROM habit), \
                 ?12, ?13, ?14, ?15, ?16)",
                params![
                    h.name,
                    h.icon,
                    h.color,
                    h.quit_at,
                    h.daily_money,
                    h.daily_minutes,
                    h.daily_frequency,
                    h.habit_since_ms,
                    h.preferred_metric.code(),
                    h.counter_type.code(),
                    h.counter_style.code(),
                    created,
                    h.kind.code(),
                    i64::from(crate::checkin::normalize_mask(i64::from(h.schedule_days))),
                    h.daily_target.max(0.0),
                    h.target_unit.trim(),
                ],
            )?;
            let id = self.conn.last_insert_rowid();
            if seed_defaults {
                // Metas por duração só fazem sentido para hábitos de parar.
                if !h.is_build() {
                    self.seed_preset_goals(id)?;
                }
                for text in DEFAULT_AFFIRMATIONS {
                    self.insert_text_item(TextTable::Affirmation, id, text)?;
                }
            }
            Ok(id)
        })
    }

    pub fn update_habit(&self, h: &Habit) -> Result<()> {
        self.conn.execute(
            "UPDATE habit SET name = ?1, icon = ?2, color = ?3, quit_at = ?4, daily_money = ?5, \
             daily_minutes = ?6, daily_frequency = ?7, preferred_metric = ?8, counter_type = ?9, \
             counter_style = ?10, schedule_days = ?11, daily_target = ?12, target_unit = ?13 WHERE id = ?14",
            params![
                h.name,
                h.icon,
                h.color,
                h.quit_at,
                h.daily_money,
                h.daily_minutes,
                h.daily_frequency,
                h.preferred_metric.code(),
                h.counter_type.code(),
                h.counter_style.code(),
                i64::from(crate::checkin::normalize_mask(i64::from(h.schedule_days))),
                h.daily_target.max(0.0),
                h.target_unit.trim(),
                h.id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_habit(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM habit WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_all_habits(&self) -> Result<()> {
        self.conn.execute("DELETE FROM habit", [])?;
        Ok(())
    }

    /// Move o hábito uma posição para cima (`-1`) ou para baixo (`+1`).
    pub fn move_habit(&self, id: i64, delta: i64) -> Result<()> {
        self.transaction(|| {
            let mut ids: Vec<i64> = self.habits()?.into_iter().map(|h| h.id).collect();
            let Some(index) = ids.iter().position(|&x| x == id) else { return Ok(()) };
            let target = index as i64 + delta;
            if target < 0 || target >= ids.len() as i64 {
                return Ok(());
            }
            ids.swap(index, target as usize);
            for (pos, hid) in ids.iter().enumerate() {
                self.conn.execute("UPDATE habit SET position = ?1 WHERE id = ?2", params![pos as i64, hid])?;
            }
            Ok(())
        })
    }

    pub fn bundle(&self, id: i64) -> Result<Option<HabitBundle>> {
        let Some(habit) = self.habit(id)? else { return Ok(None) };
        Ok(Some(HabitBundle {
            entries: self.entries(id)?,
            goals: self.goals(id)?,
            rewards: self.rewards(id)?,
            investments: self.investments(id)?,
            reasons: self.text_items(TextTable::Reason, id)?,
            affirmations: self.text_items(TextTable::Affirmation, id)?,
            rules: self.rules(id)?,
            checkins: self.checkins(id)?,
            habit,
        }))
    }

    // ----------------------------------------------------------------- Diário

    /// Mais recentes primeiro.
    pub fn entries(&self, habit_id: i64) -> Result<Vec<DiaryEntry>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM diary_entry WHERE habit_id = ?1 ORDER BY occurred_at DESC, id DESC",
        )?;
        let rows = st.query_map([habit_id], entry_from_row)?;
        rows.collect()
    }

    pub fn all_entries(&self) -> Result<Vec<DiaryEntry>> {
        let mut st = self.conn.prepare("SELECT * FROM diary_entry ORDER BY habit_id, occurred_at")?;
        let rows = st.query_map([], entry_from_row)?;
        rows.collect()
    }

    pub fn insert_entry(&self, e: &DiaryEntry) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO diary_entry (habit_id, occurred_at, money_spent, time_spent_ms, is_reset, note, rule_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                e.habit_id,
                e.occurred_at,
                e.money_spent,
                e.time_spent_ms,
                e.is_reset,
                clean_note(&e.note),
                e.rule_id
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_entry(&self, e: &DiaryEntry) -> Result<()> {
        self.conn.execute(
            "UPDATE diary_entry SET occurred_at = ?1, money_spent = ?2, time_spent_ms = ?3, \
             is_reset = ?4, note = ?5, rule_id = ?6 WHERE id = ?7",
            params![
                e.occurred_at,
                e.money_spent,
                e.time_spent_ms,
                e.is_reset,
                clean_note(&e.note),
                e.rule_id,
                e.id
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------- Check-ins

    /// Progresso por dia ("AAAA-MM-DD"), em ordem.
    pub fn checkins(&self, habit_id: i64) -> Result<Vec<DayProgress>> {
        let mut st = self.conn.prepare("SELECT day, amount FROM checkin WHERE habit_id = ?1 ORDER BY day")?;
        let rows = st.query_map([habit_id], |r| Ok(DayProgress { day: r.get(0)?, amount: r.get(1)? }))?;
        rows.collect()
    }

    /// Define o progresso total de um dia (`<= 0` apaga o registro).
    pub fn set_progress(&self, habit_id: i64, day: &str, amount: f64) -> Result<()> {
        if amount > 0.0 && amount.is_finite() {
            self.conn.execute(
                "INSERT INTO checkin (habit_id, day, amount, created_at) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(habit_id, day) DO UPDATE SET amount = excluded.amount",
                params![habit_id, day, amount, crate::now_ms()],
            )?;
        } else {
            self.conn
                .execute("DELETE FROM checkin WHERE habit_id = ?1 AND day = ?2", params![habit_id, day])?;
        }
        Ok(())
    }

    /// Soma `delta` ao progresso do dia (ex.: +10 páginas). Retorna o novo total.
    pub fn add_progress(&self, habit_id: i64, day: &str, delta: f64) -> Result<f64> {
        self.transaction(|| {
            let current: f64 = self
                .conn
                .query_row("SELECT amount FROM checkin WHERE habit_id = ?1 AND day = ?2", params![habit_id, day], |r| {
                    r.get(0)
                })
                .optional()?
                .unwrap_or(0.0);
            let total = (current + delta).max(0.0);
            self.set_progress(habit_id, day, total)?;
            Ok(total)
        })
    }

    /// Hábito sem meta: marca (`done = true`) ou desmarca um dia.
    pub fn set_checkin(&self, habit_id: i64, day: &str, done: bool) -> Result<()> {
        self.set_progress(habit_id, day, if done { 1.0 } else { 0.0 })
    }

    // ------------------------------------------------------------------- Foco

    pub fn insert_focus(&self, r: &crate::pomodoro::FocusRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO focus_session (habit_id, started_at, ended_at, duration_ms, mode) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![r.habit_id, r.started_at, r.ended_at, r.duration_ms, r.mode.code()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Sessões iniciadas a partir de `since_ms`, mais recentes primeiro.
    pub fn focus_sessions_since(&self, since_ms: i64) -> Result<Vec<crate::pomodoro::FocusRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, habit_id, started_at, ended_at, duration_ms, mode FROM focus_session \
             WHERE started_at >= ?1 ORDER BY started_at DESC",
        )?;
        let rows = st.query_map([since_ms], |r| {
            Ok(crate::pomodoro::FocusRecord {
                id: r.get(0)?,
                habit_id: r.get(1)?,
                started_at: r.get(2)?,
                ended_at: r.get(3)?,
                duration_ms: r.get(4)?,
                mode: crate::pomodoro::Mode::from_code(r.get(5)?),
            })
        })?;
        rows.collect()
    }

    pub fn delete_all_focus(&self) -> Result<()> {
        self.conn.execute("DELETE FROM focus_session", [])?;
        Ok(())
    }

    pub fn delete_focus(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM focus_session WHERE id = ?1", [id])?;
        Ok(())
    }

    // ----------------------------------------------------------------- Regras

    /// Permitidos primeiro, depois não permitidos.
    pub fn rules(&self, habit_id: i64) -> Result<Vec<HabitRule>> {
        let mut st =
            self.conn.prepare("SELECT * FROM habit_rule WHERE habit_id = ?1 ORDER BY kind, position, id")?;
        let rows = st.query_map([habit_id], rule_from_row)?;
        rows.collect()
    }

    pub fn insert_rule(&self, habit_id: i64, kind: RuleKind, text: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO habit_rule (habit_id, kind, text, position) VALUES (?1, ?2, ?3, \
             (SELECT COALESCE(MAX(position), -1) + 1 FROM habit_rule WHERE habit_id = ?1 AND kind = ?2))",
            params![habit_id, kind.code(), text.trim()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_rule(&self, id: i64, text: &str) -> Result<()> {
        self.conn.execute("UPDATE habit_rule SET text = ?1 WHERE id = ?2", params![text.trim(), id])?;
        Ok(())
    }

    /// Entradas do diário que citavam a regra ficam sem regra (o histórico não muda).
    pub fn delete_rule(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM habit_rule WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_entry(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM diary_entry WHERE id = ?1", [id])?;
        Ok(())
    }

    // ------------------------------------------------------------------ Metas

    pub fn goals(&self, habit_id: i64) -> Result<Vec<Goal>> {
        let mut st = self.conn.prepare("SELECT * FROM goal WHERE habit_id = ?1 ORDER BY id")?;
        let rows = st.query_map([habit_id], goal_from_row)?;
        rows.collect()
    }

    pub fn seed_preset_goals(&self, habit_id: i64) -> Result<()> {
        self.transaction(|| {
            for p in goal_presets() {
                self.insert_goal(&Goal::preset(habit_id, &p.label, &p.key, p.ms, &p.hex))?;
            }
            Ok(())
        })
    }

    pub fn insert_goal(&self, g: &Goal) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO goal (habit_id, name, preset_key, duration_ms, target_at, hex_color, \
             is_personal, is_forced_current) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                g.habit_id,
                g.name,
                g.preset_key,
                if g.is_personal { None } else { g.duration_ms },
                if g.is_personal { g.target_at } else { None },
                normalize_hex(&g.hex_color, "757575"),
                g.is_personal,
                g.is_forced_current,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Só metas pessoais podem ser apagadas.
    pub fn delete_personal_goal(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM goal WHERE id = ?1 AND is_personal = 1", [id])?;
        Ok(())
    }

    /// Fixa `goal_id` como meta atual (`None` remove a fixação).
    pub fn set_forced_goal(&self, habit_id: i64, goal_id: Option<i64>) -> Result<()> {
        self.transaction(|| {
            self.conn.execute("UPDATE goal SET is_forced_current = 0 WHERE habit_id = ?1", [habit_id])?;
            if let Some(gid) = goal_id {
                self.conn.execute(
                    "UPDATE goal SET is_forced_current = 1 WHERE id = ?1 AND habit_id = ?2",
                    [gid, habit_id],
                )?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ Recompensas

    pub fn rewards(&self, habit_id: i64) -> Result<Vec<Reward>> {
        let mut st = self.conn.prepare("SELECT * FROM reward WHERE habit_id = ?1 ORDER BY position, id")?;
        let rows = st.query_map([habit_id], reward_from_row)?;
        rows.collect()
    }

    pub fn insert_reward(&self, r: &Reward) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO reward (habit_id, name, price, purchased_at, archived, position) VALUES \
             (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), -1) + 1 FROM reward WHERE habit_id = ?1))",
            params![r.habit_id, r.name, r.price, r.purchased_at, r.archived],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_reward(&self, id: i64, name: &str, price: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE reward SET name = ?1, price = ?2 WHERE id = ?3 AND purchased_at IS NULL",
            params![name, price, id],
        )?;
        Ok(())
    }

    pub fn purchase_reward(&self, id: i64, at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE reward SET purchased_at = ?1 WHERE id = ?2 AND purchased_at IS NULL",
            [at, id],
        )?;
        Ok(())
    }

    /// Comprada: só arquiva (o gasto continua no saldo). Não comprada: apaga.
    pub fn remove_reward(&self, r: &Reward) -> Result<()> {
        if r.is_purchased() {
            self.conn.execute("UPDATE reward SET archived = 1 WHERE id = ?1", [r.id])?;
        } else {
            self.conn.execute("DELETE FROM reward WHERE id = ?1", [r.id])?;
        }
        Ok(())
    }

    // ---------------------------------------------------------- Investimentos

    pub fn investments(&self, habit_id: i64) -> Result<Vec<TimeInvestment>> {
        let mut st =
            self.conn.prepare("SELECT * FROM time_investment WHERE habit_id = ?1 ORDER BY position, id")?;
        let rows = st.query_map([habit_id], investment_from_row)?;
        rows.collect()
    }

    pub fn insert_investment(&self, t: &TimeInvestment) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO time_investment (habit_id, name, session_ms, invested_ms, archived, position) VALUES \
             (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), -1) + 1 FROM time_investment WHERE habit_id = ?1))",
            params![t.habit_id, t.name, t.session_ms, t.invested_ms, t.archived],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn invest(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE time_investment SET invested_ms = invested_ms + session_ms WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Com tempo investido: só arquiva (o tempo continua contando). Senão: apaga.
    pub fn remove_investment(&self, t: &TimeInvestment) -> Result<()> {
        if t.invested_ms > 0 {
            self.conn.execute("UPDATE time_investment SET archived = 1 WHERE id = ?1", [t.id])?;
        } else {
            self.conn.execute("DELETE FROM time_investment WHERE id = ?1", [t.id])?;
        }
        Ok(())
    }

    // ---------------------------------------------------- Motivos/afirmações

    pub fn text_items(&self, table: TextTable, habit_id: i64) -> Result<Vec<TextItem>> {
        let sql = format!("SELECT * FROM {} WHERE habit_id = ?1 ORDER BY position, id", table.table());
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map([habit_id], text_item_from_row)?;
        rows.collect()
    }

    pub fn insert_text_item(&self, table: TextTable, habit_id: i64, text: &str) -> Result<i64> {
        let t = table.table();
        self.conn.execute(
            &format!(
                "INSERT INTO {t} (habit_id, text, position) VALUES \
                 (?1, ?2, (SELECT COALESCE(MAX(position), -1) + 1 FROM {t} WHERE habit_id = ?1))"
            ),
            params![habit_id, text],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_text_item(&self, table: TextTable, id: i64, text: &str) -> Result<()> {
        self.conn.execute(
            &format!("UPDATE {} SET text = ?1 WHERE id = ?2", table.table()),
            params![text, id],
        )?;
        Ok(())
    }

    pub fn delete_text_item(&self, table: TextTable, id: i64) -> Result<()> {
        self.conn.execute(&format!("DELETE FROM {} WHERE id = ?1", table.table()), [id])?;
        Ok(())
    }

    // --------------------------------------------------------- Configurações

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

fn clean_note(note: &Option<String>) -> Option<String> {
    note.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn habit_from_row(r: &Row) -> Result<Habit> {
    Ok(Habit {
        id: r.get("id")?,
        name: r.get("name")?,
        icon: r.get("icon")?,
        color: r.get("color")?,
        quit_at: r.get("quit_at")?,
        daily_money: r.get("daily_money")?,
        daily_minutes: r.get("daily_minutes")?,
        daily_frequency: r.get("daily_frequency")?,
        habit_since_ms: r.get("habit_since_ms")?,
        preferred_metric: MetricType::from_code(r.get("preferred_metric")?),
        counter_type: CounterType::from_code(r.get("counter_type")?),
        counter_style: CounterStyle::from_code(r.get("counter_style")?),
        position: r.get("position")?,
        created_at: r.get("created_at")?,
        kind: HabitKind::from_code(r.get("kind")?),
        schedule_days: crate::checkin::normalize_mask(r.get("schedule_days")?),
        daily_target: r.get("daily_target")?,
        target_unit: r.get("target_unit")?,
    })
}

fn entry_from_row(r: &Row) -> Result<DiaryEntry> {
    Ok(DiaryEntry {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        occurred_at: r.get("occurred_at")?,
        money_spent: r.get("money_spent")?,
        time_spent_ms: r.get("time_spent_ms")?,
        is_reset: r.get("is_reset")?,
        note: r.get("note")?,
        rule_id: r.get("rule_id")?,
    })
}

fn rule_from_row(r: &Row) -> Result<HabitRule> {
    Ok(HabitRule {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        kind: RuleKind::from_code(r.get("kind")?),
        text: r.get("text")?,
        position: r.get("position")?,
    })
}

fn goal_from_row(r: &Row) -> Result<Goal> {
    Ok(Goal {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        name: r.get("name")?,
        preset_key: r.get("preset_key")?,
        duration_ms: r.get("duration_ms")?,
        target_at: r.get("target_at")?,
        hex_color: r.get("hex_color")?,
        is_personal: r.get("is_personal")?,
        is_forced_current: r.get("is_forced_current")?,
    })
}

fn reward_from_row(r: &Row) -> Result<Reward> {
    Ok(Reward {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        name: r.get("name")?,
        price: r.get("price")?,
        purchased_at: r.get("purchased_at")?,
        archived: r.get("archived")?,
        position: r.get("position")?,
    })
}

fn investment_from_row(r: &Row) -> Result<TimeInvestment> {
    Ok(TimeInvestment {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        name: r.get("name")?,
        session_ms: r.get("session_ms")?,
        invested_ms: r.get("invested_ms")?,
        archived: r.get("archived")?,
        position: r.get("position")?,
    })
}

fn text_item_from_row(r: &Row) -> Result<TextItem> {
    Ok(TextItem {
        id: r.get("id")?,
        habit_id: r.get("habit_id")?,
        text: r.get("text")?,
        position: r.get("position")?,
    })
}

const SCHEMA_V1: &str = r#"
CREATE TABLE habit (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    icon             INTEGER NOT NULL DEFAULT 2001,
    color            INTEGER NOT NULL DEFAULT 1008,
    quit_at          INTEGER NOT NULL,
    daily_money      REAL    NOT NULL DEFAULT 0 CHECK (daily_money   >= 0),
    daily_minutes    REAL    NOT NULL DEFAULT 0 CHECK (daily_minutes >= 0),
    daily_frequency  REAL    NOT NULL DEFAULT 0 CHECK (daily_frequency >= 0),
    habit_since_ms   INTEGER,
    preferred_metric INTEGER NOT NULL DEFAULT 0,
    counter_type     INTEGER NOT NULL DEFAULT 300,
    counter_style    INTEGER NOT NULL DEFAULT 400,
    position         INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL
);

CREATE TABLE diary_entry (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id      INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    occurred_at   INTEGER NOT NULL,
    money_spent   REAL    NOT NULL DEFAULT 0 CHECK (money_spent >= 0),
    time_spent_ms INTEGER NOT NULL DEFAULT 0 CHECK (time_spent_ms >= 0),
    is_reset      INTEGER NOT NULL DEFAULT 1 CHECK (is_reset IN (0,1)),
    note          TEXT
);
CREATE INDEX ix_diary_habit_reset_ts ON diary_entry (habit_id, is_reset, occurred_at);

CREATE TABLE goal (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id          INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    preset_key        TEXT,
    duration_ms       INTEGER,
    target_at         INTEGER,
    hex_color         TEXT    NOT NULL,
    is_personal       INTEGER NOT NULL DEFAULT 0 CHECK (is_personal IN (0,1)),
    is_forced_current INTEGER NOT NULL DEFAULT 0 CHECK (is_forced_current IN (0,1)),
    CHECK ((is_personal = 0 AND duration_ms IS NOT NULL) OR (is_personal = 1 AND target_at IS NOT NULL))
);
CREATE INDEX ix_goal_habit ON goal (habit_id);
CREATE UNIQUE INDEX ux_goal_one_forced ON goal (habit_id) WHERE is_forced_current = 1;

CREATE TABLE trophy (
    habit_id    INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    trophy_id   INTEGER NOT NULL,
    achieved_at INTEGER,
    notified    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (habit_id, trophy_id)
);

CREATE TABLE reward (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    price        REAL    NOT NULL CHECK (price >= 0),
    purchased_at INTEGER,
    archived     INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_reward_habit ON reward (habit_id);

CREATE TABLE time_investment (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id    INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    session_ms  INTEGER NOT NULL CHECK (session_ms > 0),
    invested_ms INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    position    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_investment_habit ON time_investment (habit_id);

CREATE TABLE reason (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    text     TEXT    NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_reason_habit ON reason (habit_id);

CREATE TABLE affirmation (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    text     TEXT    NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_affirmation_habit ON affirmation (habit_id);

CREATE TABLE setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// v2: regras do hábito (permitido / não permitido) e vínculo da entrada do diário com a regra.
const MIGRATION_V2: &str = r#"
CREATE TABLE habit_rule (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    kind     INTEGER NOT NULL CHECK (kind IN (0,1)),
    text     TEXT    NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_rule_habit ON habit_rule (habit_id, kind);

ALTER TABLE diary_entry ADD COLUMN rule_id INTEGER REFERENCES habit_rule(id) ON DELETE SET NULL;
"#;

/// v3: hábitos de check diário (tipo + dias programados) e tabela de check-ins.
const MIGRATION_V3: &str = r#"
ALTER TABLE habit ADD COLUMN kind INTEGER NOT NULL DEFAULT 0;
ALTER TABLE habit ADD COLUMN schedule_days INTEGER NOT NULL DEFAULT 127;
ALTER TABLE habit ADD COLUMN daily_target REAL NOT NULL DEFAULT 0;
ALTER TABLE habit ADD COLUMN target_unit TEXT NOT NULL DEFAULT '';

CREATE TABLE checkin (
    habit_id   INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    day        TEXT    NOT NULL,
    amount     REAL    NOT NULL DEFAULT 1 CHECK (amount > 0),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (habit_id, day)
);

CREATE TABLE focus_session (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id    INTEGER REFERENCES habit(id) ON DELETE SET NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL CHECK (duration_ms > 0),
    mode        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_focus_started ON focus_session (started_at);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::MS_PER_DAY;

    const BASE: i64 = 1_735_689_600_000;

    #[test]
    fn regras_e_vinculo_com_o_diario() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Refrigerante", BASE), false).unwrap();
        let zero = db.insert_rule(id, RuleKind::Allowed, "Refrigerante zero").unwrap();
        let comum = db.insert_rule(id, RuleKind::Forbidden, " Refrigerante comum ").unwrap();
        db.insert_rule(id, RuleKind::Allowed, "Água com gás").unwrap();

        let rules = db.rules(id).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].kind, RuleKind::Allowed);
        assert_eq!(rules[2].text, "Refrigerante comum");

        let entry_id = db
            .insert_entry(&DiaryEntry { is_reset: false, rule_id: Some(zero), ..DiaryEntry::reset(id, BASE + MS_PER_DAY) })
            .unwrap();
        db.insert_entry(&DiaryEntry { rule_id: Some(comum), ..DiaryEntry::reset(id, BASE + 2 * MS_PER_DAY) })
            .unwrap();
        let bundle = db.bundle(id).unwrap().unwrap();
        assert_eq!(bundle.rule_usage(zero), 1);
        assert_eq!(bundle.rule(Some(comum)).map(|r| r.kind), Some(RuleKind::Forbidden));

        // Apagar a regra mantém a entrada, só sem vínculo.
        db.delete_rule(zero).unwrap();
        let entries = db.entries(id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().find(|e| e.id == entry_id).unwrap().rule_id, None);

        // Apagar o hábito apaga as regras.
        db.delete_habit(id).unwrap();
        assert!(db.rules(id).unwrap().is_empty());
    }

    #[test]
    fn habito_de_check_diario() {
        let db = Db::open_in_memory().unwrap();
        let mut h = Habit::new("Acordar cedo", BASE);
        h.kind = HabitKind::Build;
        h.schedule_days = crate::checkin::WEEKDAYS;
        let id = db.insert_habit(&h, true).unwrap();
        let saved = db.habit(id).unwrap().unwrap();
        assert_eq!(saved.kind, HabitKind::Build);
        assert_eq!(saved.schedule_days, crate::checkin::WEEKDAYS);
        assert!(db.goals(id).unwrap().is_empty(), "hábito de check não recebe metas por duração");

        db.set_checkin(id, "2025-01-06", true).unwrap();
        db.set_checkin(id, "2025-01-06", true).unwrap(); // idempotente
        db.set_checkin(id, "2025-01-07", true).unwrap();
        db.set_checkin(id, "2025-01-07", false).unwrap();
        let days: Vec<String> = db.checkins(id).unwrap().into_iter().map(|p| p.day).collect();
        assert_eq!(days, vec!["2025-01-06".to_string()]);

        // Meta diária: progresso acumulado.
        let mut reading = Habit::new("Leitura", BASE);
        reading.kind = HabitKind::Build;
        reading.daily_target = 30.0;
        reading.target_unit = " páginas ".into();
        let rid = db.insert_habit(&reading, true).unwrap();
        let saved_reading = db.habit(rid).unwrap().unwrap();
        assert!(saved_reading.has_target());
        assert_eq!(saved_reading.target_unit, "páginas");
        assert_eq!(db.add_progress(rid, "2025-01-06", 10.0).unwrap(), 10.0);
        assert_eq!(db.add_progress(rid, "2025-01-06", 25.0).unwrap(), 35.0);
        assert_eq!(db.add_progress(rid, "2025-01-06", -50.0).unwrap(), 0.0);
        assert!(db.checkins(rid).unwrap().is_empty(), "progresso zerado apaga o registro");
        db.set_progress(rid, "2025-01-07", 30.0).unwrap();
        let bundle = db.bundle(rid).unwrap().unwrap();
        let day7 = crate::checkin::parse_day("2025-01-07").unwrap();
        assert_eq!(bundle.amount_on(day7), 30.0);
        assert!(bundle.checkin_stats(day7).done_today);

        let mut edited = saved.clone();
        edited.schedule_days = 0; // inválido: vira todos os dias
        db.update_habit(&edited).unwrap();
        assert_eq!(db.habit(id).unwrap().unwrap().schedule_days, crate::checkin::ALL_DAYS);

        db.delete_habit(id).unwrap();
        assert!(db.checkins(id).unwrap().is_empty());
    }

    #[test]
    fn sessoes_de_foco() {
        use crate::pomodoro::{FocusRecord, Mode};
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Estudar", BASE), false).unwrap();
        let rec = |start: i64, habit: Option<i64>| FocusRecord {
            id: 0,
            habit_id: habit,
            started_at: start,
            ended_at: start + 1_500_000,
            duration_ms: 1_500_000,
            mode: Mode::Pomodoro,
        };
        db.insert_focus(&rec(BASE, Some(id))).unwrap();
        db.insert_focus(&rec(BASE + MS_PER_DAY, None)).unwrap();
        let all = db.focus_sessions_since(BASE).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].started_at, BASE + MS_PER_DAY, "mais recentes primeiro");
        assert_eq!(db.focus_sessions_since(BASE + 1).unwrap().len(), 1);
        db.delete_habit(id).unwrap();
        assert_eq!(db.focus_sessions_since(BASE).unwrap()[1].habit_id, None, "sessão fica, sem hábito");
        db.delete_focus(all[0].id).unwrap();
        assert_eq!(db.focus_sessions_since(0).unwrap().len(), 1);
    }

    #[test]
    fn versao_do_esquema() {
        let db = Db::open_in_memory().unwrap();
        let v: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn novo_habito_recebe_metas_e_afirmacoes() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Café", BASE), true).unwrap();
        assert_eq!(db.goals(id).unwrap().len(), goal_presets().len());
        assert_eq!(db.text_items(TextTable::Affirmation, id).unwrap().len(), DEFAULT_AFFIRMATIONS.len());
        assert_eq!(db.habit(id).unwrap().unwrap().name, "Café");
    }

    #[test]
    fn apagar_habito_apaga_dados() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Café", BASE), true).unwrap();
        db.insert_entry(&DiaryEntry::reset(id, BASE + MS_PER_DAY)).unwrap();
        db.delete_habit(id).unwrap();
        assert!(db.habits().unwrap().is_empty());
        assert!(db.all_entries().unwrap().is_empty());
        assert!(db.goals(id).unwrap().is_empty());
    }

    #[test]
    fn uma_meta_fixada_por_habito() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Café", BASE), true).unwrap();
        let goals = db.goals(id).unwrap();
        db.set_forced_goal(id, Some(goals[3].id)).unwrap();
        db.set_forced_goal(id, Some(goals[5].id)).unwrap();
        let forced: Vec<_> = db.goals(id).unwrap().into_iter().filter(|g| g.is_forced_current).collect();
        assert_eq!(forced.len(), 1);
        assert_eq!(forced[0].id, goals[5].id);
    }

    #[test]
    fn reordenar() {
        let db = Db::open_in_memory().unwrap();
        let a = db.insert_habit(&Habit::new("A", BASE), false).unwrap();
        let b = db.insert_habit(&Habit::new("B", BASE), false).unwrap();
        db.move_habit(b, -1).unwrap();
        let ids: Vec<i64> = db.habits().unwrap().iter().map(|h| h.id).collect();
        assert_eq!(ids, vec![b, a]);
        db.move_habit(b, -1).unwrap(); // já é o primeiro: nada muda
        assert_eq!(db.habits().unwrap()[0].id, b);
    }

    #[test]
    fn recompensas_e_investimentos() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Café", BASE), false).unwrap();
        let rid = db.insert_reward(&Reward::new(id, "Livro", 50.0)).unwrap();
        db.purchase_reward(rid, BASE).unwrap();
        let r = db.rewards(id).unwrap().remove(0);
        assert_eq!(r.purchased_at, Some(BASE));
        db.remove_reward(&r).unwrap();
        assert!(db.rewards(id).unwrap()[0].archived);

        let tid = db.insert_investment(&TimeInvestment::new(id, "Leitura", 1_800_000)).unwrap();
        db.invest(tid).unwrap();
        db.invest(tid).unwrap();
        assert_eq!(db.investments(id).unwrap()[0].invested_ms, 3_600_000);
    }

    #[test]
    fn configuracoes() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.setting("currency").unwrap(), None);
        db.set_setting("currency", "EUR").unwrap();
        db.set_setting("currency", "BRL").unwrap();
        assert_eq!(db.setting("currency").unwrap().as_deref(), Some("BRL"));
    }
}
