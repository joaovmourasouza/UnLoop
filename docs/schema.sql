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

-- v2: regras do hábito. kind 0 = permitido (não reinicia), 1 = não permitido (reinicia).
CREATE TABLE IF NOT EXISTS habit_rule (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    kind     INTEGER NOT NULL CHECK (kind IN (0,1)),
    text     TEXT    NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_rule_habit ON habit_rule (habit_id, kind);
-- diary_entry ganha: rule_id INTEGER REFERENCES habit_rule(id) ON DELETE SET NULL
-- Uma entrada ligada a uma regra "não permitido" reinicia a sequência; "permitido" não.

-- v3: hábitos de check diário e foco.
-- habit ganha: kind INTEGER (0 = parar/contador, 1 = criar/check diário),
--              schedule_days INTEGER (bit 0 = segunda … bit 6 = domingo; 127 = todos),
--              daily_target REAL (0 = só marcar), target_unit TEXT
CREATE TABLE IF NOT EXISTS checkin (
    habit_id   INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    day        TEXT    NOT NULL,                 -- AAAA-MM-DD (data local)
    amount     REAL    NOT NULL DEFAULT 1 CHECK (amount > 0),  -- progresso do dia
    created_at INTEGER NOT NULL,
    PRIMARY KEY (habit_id, day)
);
-- v5: dias marcados como "não feito" (quebram a sequência, inclusive hoje).
-- Marcar apaga o progresso do dia; registrar progresso apaga a marca.
CREATE TABLE IF NOT EXISTS checkin_miss (
    habit_id   INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    day        TEXT    NOT NULL,                 -- AAAA-MM-DD (data local)
    created_at INTEGER NOT NULL,
    PRIMARY KEY (habit_id, day)
);
CREATE TABLE IF NOT EXISTS focus_session (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    habit_id    INTEGER REFERENCES habit(id) ON DELETE SET NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL CHECK (duration_ms > 0),
    mode        INTEGER NOT NULL DEFAULT 0      -- 0 = Pomodoro, 1 = cronômetro
);

-- Configurações chave/valor (substitui o DataStore).
CREATE TABLE IF NOT EXISTS setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Início da sequência atual (equivalente ao COALESCE do original):
-- SELECT COALESCE((SELECT MAX(occurred_at) FROM diary_entry WHERE habit_id = :id AND is_reset = 1),
--                 (SELECT quit_at FROM habit WHERE id = :id));

-- v6: finanças (funções do FinanCerto). Meses "AAAA-MM", datas "AAAA-MM-DD".
-- Regras em crates/unloop-core/src/finance.rs. A migração também cria as
-- categorias padrão (Salário, Mercado, Moradia…).
CREATE TABLE IF NOT EXISTS fin_account (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT    NOT NULL UNIQUE,
    initial_balance REAL    NOT NULL DEFAULT 0,
    color           INTEGER NOT NULL DEFAULT 1003,
    position        INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fin_category (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    name  TEXT    NOT NULL,
    kind  INTEGER NOT NULL,
    color INTEGER NOT NULL DEFAULT 1021,
    UNIQUE (name, kind)
);

CREATE TABLE IF NOT EXISTS fin_card (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL,
    credit_limit REAL    NOT NULL DEFAULT 0,
    closing_day  INTEGER NOT NULL DEFAULT 3,
    due_day      INTEGER NOT NULL DEFAULT 10,
    account_id   INTEGER REFERENCES fin_account(id) ON DELETE SET NULL,
    color        INTEGER NOT NULL DEFAULT 1005,
    created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fin_entry (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    month             TEXT    NOT NULL,
    kind              INTEGER NOT NULL,
    description       TEXT    NOT NULL,
    amount            REAL    NOT NULL,
    due_date          TEXT,
    category_id       INTEGER REFERENCES fin_category(id) ON DELETE SET NULL,
    account_id        INTEGER REFERENCES fin_account(id) ON DELETE SET NULL,
    paid              INTEGER NOT NULL DEFAULT 0,
    paid_date         TEXT,
    note              TEXT    NOT NULL DEFAULT '',
    excluded          INTEGER NOT NULL DEFAULT 0,
    recurrence_group  TEXT,
    recurrence_until  TEXT,
    installment_group TEXT,
    installment_num   INTEGER,
    installment_total INTEGER,
    transfer_group    TEXT,
    card_id           INTEGER REFERENCES fin_card(id) ON DELETE SET NULL,
    created_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_fin_entry_month ON fin_entry (month, kind);
CREATE UNIQUE INDEX IF NOT EXISTS ux_fin_entry_recurrence ON fin_entry (recurrence_group, month)
    WHERE recurrence_group IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS ux_fin_entry_invoice ON fin_entry (card_id, month) WHERE card_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS fin_recurrence_skip (
    recurrence_group TEXT NOT NULL,
    month            TEXT NOT NULL,
    PRIMARY KEY (recurrence_group, month)
);

CREATE TABLE IF NOT EXISTS fin_budget (
    category_id   INTEGER PRIMARY KEY REFERENCES fin_category(id) ON DELETE CASCADE,
    monthly_limit REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS fin_card_purchase (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id           INTEGER NOT NULL REFERENCES fin_card(id) ON DELETE CASCADE,
    description       TEXT    NOT NULL,
    amount            REAL    NOT NULL,
    date              TEXT    NOT NULL,
    invoice_month     TEXT    NOT NULL,
    category_id       INTEGER REFERENCES fin_category(id) ON DELETE SET NULL,
    installment_group TEXT,
    installment_num   INTEGER,
    installment_total INTEGER,
    created_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_fin_purchase_invoice ON fin_card_purchase (card_id, invoice_month);

CREATE TABLE IF NOT EXISTS fin_goal (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    target     REAL    NOT NULL,
    deadline   TEXT,
    start      TEXT    NOT NULL,
    state      INTEGER NOT NULL DEFAULT 0,
    done_at    INTEGER,
    icon       TEXT    NOT NULL DEFAULT '🎯',
    color      INTEGER NOT NULL DEFAULT 1008,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fin_goal_deposit (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    goal_id    INTEGER NOT NULL REFERENCES fin_goal(id) ON DELETE CASCADE,
    amount     REAL    NOT NULL,
    date       TEXT    NOT NULL,
    note       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_fin_deposit_goal ON fin_goal_deposit (goal_id, date);

-- Metas em cascata (ano → semestre → trimestre → mês → semana). Sem número de versão:
-- o app cria estas tabelas (e task.objective_id) sempre que faltam.
CREATE TABLE IF NOT EXISTS objective (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id   INTEGER REFERENCES objective(id) ON DELETE CASCADE,  -- meta-mãe (nível acima)
    level       INTEGER NOT NULL,              -- 0 ano, 1 semestre, 2 trimestre, 3 mês, 4 semana
    period      TEXT    NOT NULL,              -- 2026 | 2026-S2 | 2026-T4 | 2026-11 | 2026-W47 (ISO)
    title       TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    area        TEXT    NOT NULL DEFAULT '',
    status      INTEGER NOT NULL DEFAULT 0,    -- 0 em andamento, 1 concluída, 2 riscada
    status_note TEXT    NOT NULL DEFAULT '',   -- motivo (ex.: "remanejado pra W48")
    position    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    closed_at   INTEGER
);
CREATE TABLE IF NOT EXISTS objective_habit (
    objective_id INTEGER NOT NULL REFERENCES objective(id) ON DELETE CASCADE,
    habit_id     INTEGER NOT NULL REFERENCES habit(id) ON DELETE CASCADE,
    PRIMARY KEY (objective_id, habit_id)
);
-- task ganha: objective_id INTEGER REFERENCES objective(id) ON DELETE SET NULL
