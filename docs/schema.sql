-- UnLoop — esquema SQLite proposto (derivado da análise do Quitzilla 3.0.2; ver REA-Quitzilla.md)
-- Convenções: datas em epoch milissegundos UTC; enums armazenados com os códigos inteiros do original.

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

-- Versão do esquema: PRAGMA user_version (ver lib/data/database.dart, fonte da verdade).

CREATE TABLE IF NOT EXISTS habit (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    icon             INTEGER NOT NULL DEFAULT 2001,          -- 2001..2059
    color            INTEGER NOT NULL DEFAULT 1003,          -- 1001..1021
    quit_at          INTEGER NOT NULL,                       -- início da jornada (quitDate)
    daily_money      REAL    NOT NULL DEFAULT 0 CHECK (daily_money   >= 0),
    daily_minutes    REAL    NOT NULL DEFAULT 0 CHECK (daily_minutes >= 0),
    daily_frequency  REAL    NOT NULL DEFAULT 0 CHECK (daily_frequency >= 0),
    habit_since_ms   INTEGER,                                -- "há quanto tempo tem o hábito" (onboarding)
    preferred_metric INTEGER NOT NULL DEFAULT 0,             -- 0=MONEY 1=TIME
    counter_type     INTEGER NOT NULL DEFAULT 300,           -- 300=ABSTINENCE 200=REMAINING
    counter_style    INTEGER NOT NULL DEFAULT 400,           -- 400..412
    position         INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL
);

-- Diário. is_reset=1 = recaída (reinicia o contador); 0 = anotação/deslize sem reset.
CREATE TABLE IF NOT EXISTS diary_entry (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    occurred_at  INTEGER NOT NULL,
    money_spent  REAL    NOT NULL DEFAULT 0 CHECK (money_spent >= 0),
    time_spent_ms INTEGER NOT NULL DEFAULT 0 CHECK (time_spent_ms >= 0),
    is_reset     INTEGER NOT NULL DEFAULT 1 CHECK (is_reset IN (0,1)),
    note         TEXT
);
CREATE INDEX IF NOT EXISTS ix_diary_habit_reset_ts ON diary_entry (habit_id, is_reset, occurred_at);

-- Metas: preset => duration_ms relativo ao início da sequência; pessoal => target_at absoluto.
CREATE TABLE IF NOT EXISTS goal (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id      INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    preset_key    TEXT,                                      -- ex.: 'DAY_7', NULL se pessoal
    duration_ms   INTEGER,
    target_at     INTEGER,
    hex_color     TEXT    NOT NULL,
    is_personal   INTEGER NOT NULL DEFAULT 0 CHECK (is_personal IN (0,1)),
    is_forced_current INTEGER NOT NULL DEFAULT 0 CHECK (is_forced_current IN (0,1)),
    CHECK ((is_personal = 0 AND duration_ms IS NOT NULL) OR (is_personal = 1 AND target_at IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS ix_goal_habit ON goal (habit_id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_goal_one_forced ON goal (habit_id) WHERE is_forced_current = 1;

-- Troféus: derivável; a tabela registra conquista/notificação. trophy_id 1..11 (1=DECISION_MADE).
CREATE TABLE IF NOT EXISTS trophy (
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    trophy_id    INTEGER NOT NULL,
    achieved_at  INTEGER,
    notified     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (habit_id, trophy_id)
);

CREATE TABLE IF NOT EXISTS reward (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    price        REAL    NOT NULL CHECK (price >= 0),
    purchased_at INTEGER,                                    -- NULL = não comprada
    archived     INTEGER NOT NULL DEFAULT 0,                 -- "apagada" mas ainda conta no saldo se comprada
    position     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_reward_habit ON reward (habit_id);

CREATE TABLE IF NOT EXISTS time_investment (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    session_ms   INTEGER NOT NULL CHECK (session_ms > 0),
    invested_ms  INTEGER NOT NULL DEFAULT 0,                 -- soma de todas as sessões
    archived     INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_investment_habit ON time_investment (habit_id);

CREATE TABLE IF NOT EXISTS reason (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id  INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    text      TEXT    NOT NULL,
    position  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_reason_habit ON reason (habit_id);

CREATE TABLE IF NOT EXISTS affirmation (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id  INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    text      TEXT    NOT NULL,
    position  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_affirmation_habit ON affirmation (habit_id);

-- Configurações chave/valor (substitui o DataStore).
CREATE TABLE IF NOT EXISTS setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Início da sequência atual (equivalente ao COALESCE do original):
-- SELECT COALESCE((SELECT MAX(occurred_at) FROM diary_entry WHERE habit_id = :id AND is_reset = 1),
--                 (SELECT quit_at FROM habit WHERE id = :id));
