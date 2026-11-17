//! Finanças pessoais (funções trazidas do FinanCerto, sem a parte de servidor):
//! contas com saldo, categorias, lançamentos avulsos, recorrentes e parcelados,
//! transferências entre contas, cartões de crédito com fatura, orçamento por
//! categoria e metas de economia com depósitos.
//!
//! Regras principais:
//! - Meses são "AAAA-MM" e datas são "AAAA-MM-DD" (texto, horário local).
//! - Lançamento recorrente forma uma série (`recurrence_group`). As ocorrências
//!   são criadas sozinhas quando um mês é aberto, copiando a ocorrência anterior
//!   mais recente (assim mudanças de valor seguem adiante). Apagar uma ocorrência
//!   registra um "pulo" para ela não voltar.
//! - Transferência são dois lançamentos pagos (saída e entrada) ligados por
//!   `transfer_group`; não contam como receita nem despesa.
//! - "Fora dos totais" (`excluded`): mexe no saldo da conta, mas não nos totais.
//! - Cartão: cada compra cai na fatura do mês do vencimento (pelo dia de
//!   fechamento). A fatura vira um lançamento de despesa (`card_id`) mantido em
//!   sincronia com a soma das compras; pagá-la é marcar esse lançamento como pago.
//!   Nos gastos por categoria e no orçamento entram as compras, não a fatura.
//! - Saldo da conta = saldo inicial + entradas pagas − saídas pagas.

use std::collections::HashMap;

use chrono::{Datelike, Days, NaiveDate};
use rusqlite::{params, OptionalExtension, Result, Row};
use serde_json::{json, Value};

use crate::db::Db;

// ------------------------------------------------------------------- Meses

pub const MONTH_NAMES: [&str; 12] = [
    "Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro",
    "Novembro", "Dezembro",
];
pub const MONTH_SHORT: [&str; 12] = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];

pub fn month_of(day: NaiveDate) -> String {
    day.format("%Y-%m").to_string()
}

pub fn current_month() -> String {
    month_of(crate::checkin::today())
}

pub fn parse_month(month: &str) -> Option<(i32, u32)> {
    let (y, m) = month.trim().split_once('-')?;
    let (y, m) = (y.parse::<i32>().ok()?, m.get(..2).unwrap_or(m).parse::<u32>().ok()?);
    (1..=12).contains(&m).then_some((y, m))
}

pub fn add_months(month: &str, n: i32) -> String {
    let (y, m) = parse_month(month).unwrap_or((2000, 1));
    let index = y * 12 + (m as i32 - 1) + n;
    format!("{:04}-{:02}", index.div_euclid(12), index.rem_euclid(12) + 1)
}

/// Meses de `from` até `to` (negativo se `to` vem antes).
pub fn months_between(from: &str, to: &str) -> Option<i64> {
    let (y1, m1) = parse_month(from)?;
    let (y2, m2) = parse_month(to)?;
    Some(i64::from(y2 - y1) * 12 + i64::from(m2) - i64::from(m1))
}

pub fn days_in_month(month: &str) -> u32 {
    let (y, m) = parse_month(month).unwrap_or((2000, 1));
    let first = NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_default();
    let next = first.checked_add_months(chrono::Months::new(1)).unwrap_or(first);
    (next - first).num_days() as u32
}

pub fn first_day(month: &str) -> NaiveDate {
    let (y, m) = parse_month(month).unwrap_or((2000, 1));
    NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_default()
}

/// Dia `day` no mês, limitado ao último dia (31 em fevereiro vira 28/29).
pub fn day_in_month(month: &str, day: u32) -> String {
    let (y, m) = parse_month(month).unwrap_or((2000, 1));
    format!("{y:04}-{m:02}-{:02}", day.clamp(1, days_in_month(month)))
}

/// "Novembro de 2026".
pub fn month_label(month: &str) -> String {
    match parse_month(month) {
        Some((y, m)) => format!("{} de {y}", MONTH_NAMES[m as usize - 1]),
        None => month.to_string(),
    }
}

/// "nov/26".
pub fn month_short(month: &str) -> String {
    match parse_month(month) {
        Some((y, m)) => format!("{}/{:02}", MONTH_SHORT[m as usize - 1], y.rem_euclid(100)),
        None => month.to_string(),
    }
}

pub fn parse_date(text: &str) -> Option<NaiveDate> {
    crate::checkin::parse_day(text)
}

/// "AAAA-MM-DD" → "DD/MM".
pub fn date_short(text: &str) -> String {
    parse_date(text).map_or_else(|| text.to_string(), |d| d.format("%d/%m").to_string())
}

/// Mês da fatura (o do vencimento) em que cai uma compra feita em `date`.
pub fn invoice_month(date: NaiveDate, closing_day: u32, due_day: u32) -> String {
    let month = month_of(date);
    let closing_month = if date.day() > closing_day { add_months(&month, 1) } else { month };
    if due_day > closing_day {
        closing_month
    } else {
        add_months(&closing_month, 1)
    }
}

/// Divide um total em `n` parcelas em centavos; a última absorve o arredondamento.
pub fn split_installments(total: f64, n: u32) -> Vec<f64> {
    let n = n.max(1);
    let cents = (total * 100.0).round() as i64;
    let each = cents / i64::from(n);
    let mut parts = vec![each as f64 / 100.0; n as usize];
    if let Some(last) = parts.last_mut() {
        *last = (cents - each * (i64::from(n) - 1)) as f64 / 100.0;
    }
    parts
}

// ------------------------------------------------------------------ Modelo

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Income,
    Expense,
}

impl Kind {
    pub fn code(self) -> i64 {
        match self {
            Kind::Income => 0,
            Kind::Expense => 1,
        }
    }
    pub fn from_code(code: i64) -> Kind {
        if code == 0 {
            Kind::Income
        } else {
            Kind::Expense
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Income => "Receita",
            Kind::Expense => "Despesa",
        }
    }
    pub fn sign(self) -> f64 {
        match self {
            Kind::Income => 1.0,
            Kind::Expense => -1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub initial_balance: f64,
    pub color: i64,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountBalance {
    pub account: Account,
    /// Saldo inicial + tudo o que já foi pago/recebido.
    pub current: f64,
    /// `current` + pendências até o fim do mês consultado.
    pub forecast: f64,
    pub month_income: f64,
    pub month_expense: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub kind: Kind,
    pub color: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub month: String,
    pub kind: Kind,
    pub description: String,
    pub amount: f64,
    pub due_date: Option<String>,
    pub category_id: Option<i64>,
    pub account_id: Option<i64>,
    pub paid: bool,
    pub paid_date: Option<String>,
    pub note: String,
    /// Fora dos totais (dinheiro que só passa pela conta).
    pub excluded: bool,
    pub recurrence_group: Option<String>,
    /// Último mês da série (`None` = sem fim).
    pub recurrence_until: Option<String>,
    pub installment_group: Option<String>,
    pub installment_num: Option<i64>,
    pub installment_total: Option<i64>,
    pub transfer_group: Option<String>,
    /// Lançamento da fatura deste cartão.
    pub card_id: Option<i64>,
    pub created_at: i64,
}

impl Entry {
    pub fn new(kind: Kind, description: &str, amount: f64, month: &str) -> Self {
        Entry {
            id: 0,
            month: month.to_string(),
            kind,
            description: description.trim().to_string(),
            amount,
            due_date: None,
            category_id: None,
            account_id: None,
            paid: false,
            paid_date: None,
            note: String::new(),
            excluded: false,
            recurrence_group: None,
            recurrence_until: None,
            installment_group: None,
            installment_num: None,
            installment_total: None,
            transfer_group: None,
            card_id: None,
            created_at: 0,
        }
    }
    pub fn is_transfer(&self) -> bool {
        self.transfer_group.is_some()
    }
    pub fn is_recurring(&self) -> bool {
        self.recurrence_group.is_some()
    }
    pub fn is_installment(&self) -> bool {
        self.installment_total.is_some_and(|t| t > 1)
    }
    pub fn is_invoice(&self) -> bool {
        self.card_id.is_some()
    }
    /// Entra nos totais de receitas/despesas.
    pub fn counts(&self) -> bool {
        !self.is_transfer() && !self.excluded
    }
    /// Data mostrada na lista e no calendário: vencimento, senão a do pagamento.
    pub fn effective_date(&self) -> Option<&str> {
        self.due_date.as_deref().filter(|d| !d.is_empty()).or(self.paid_date.as_deref().filter(|d| !d.is_empty()))
    }
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        !self.paid
            && self.kind == Kind::Expense
            && self.due_date.as_deref().and_then(parse_date).is_some_and(|d| d < today)
    }
}

/// Como um lançamento novo se repete.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    Once,
    /// Todo mês; `Some(n)` para parar depois de `n` meses.
    Monthly(Option<u32>),
    /// `n` parcelas de `amount` cada, uma por mês.
    Installments(u32),
}

/// Até onde vale uma alteração/remoção num lançamento de série.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    This,
    /// Esta ocorrência e as seguintes (série recorrente ou parcelas).
    Following,
    /// A série/parcelamento inteiro.
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: i64,
    pub name: String,
    pub credit_limit: f64,
    pub closing_day: u32,
    pub due_day: u32,
    pub account_id: Option<i64>,
    pub color: i64,
}

impl Card {
    pub fn new(name: &str) -> Self {
        Card { id: 0, name: name.trim().to_string(), credit_limit: 0.0, closing_day: 3, due_day: 10, account_id: None, color: 1005 }
    }
    pub fn invoice_for(&self, date: NaiveDate) -> String {
        invoice_month(date, self.closing_day, self.due_day)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardPurchase {
    pub id: i64,
    pub card_id: i64,
    pub description: String,
    pub amount: f64,
    pub date: String,
    pub invoice_month: String,
    pub category_id: Option<i64>,
    pub installment_group: Option<String>,
    pub installment_num: Option<i64>,
    pub installment_total: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardSummary {
    pub card: Card,
    /// Fatura do mês consultado.
    pub invoice: f64,
    pub invoice_paid: bool,
    /// Soma das faturas ainda não pagas (limite comprometido).
    pub open_total: f64,
}

impl CardSummary {
    pub fn usage(&self) -> f64 {
        if self.card.credit_limit > 0.0 {
            self.open_total / self.card.credit_limit
        } else {
            0.0
        }
    }
    pub fn available(&self) -> f64 {
        self.card.credit_limit - self.open_total
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BudgetLine {
    pub category: Category,
    pub limit: f64,
    pub spent: f64,
}

impl BudgetLine {
    pub fn fraction(&self) -> f64 {
        if self.limit > 0.0 {
            self.spent / self.limit
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalState {
    Active,
    Paused,
    Done,
    Archived,
}

impl GoalState {
    pub const ALL: [GoalState; 4] = [GoalState::Active, GoalState::Paused, GoalState::Done, GoalState::Archived];
    pub fn code(self) -> i64 {
        match self {
            GoalState::Active => 0,
            GoalState::Paused => 1,
            GoalState::Done => 2,
            GoalState::Archived => 3,
        }
    }
    pub fn from_code(code: i64) -> GoalState {
        match code {
            1 => GoalState::Paused,
            2 => GoalState::Done,
            3 => GoalState::Archived,
            _ => GoalState::Active,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            GoalState::Active => "Ativa",
            GoalState::Paused => "Pausada",
            GoalState::Done => "Concluída",
            GoalState::Archived => "Arquivada",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: i64,
    pub name: String,
    pub target: f64,
    /// Soma dos depósitos.
    pub saved: f64,
    /// Mês-limite ("AAAA-MM").
    pub deadline: Option<String>,
    /// Mês em que a meta começou.
    pub start: String,
    pub state: GoalState,
    pub done_at: Option<i64>,
    pub icon: String,
    pub color: i64,
}

impl Goal {
    pub fn new(name: &str, target: f64) -> Self {
        Goal {
            id: 0,
            name: name.trim().to_string(),
            target,
            saved: 0.0,
            deadline: None,
            start: current_month(),
            state: GoalState::Active,
            done_at: None,
            icon: "🎯".into(),
            color: 1008,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GoalDeposit {
    pub id: i64,
    pub goal_id: i64,
    pub amount: f64,
    pub date: String,
    pub note: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pace {
    Done,
    Paused,
    Archived,
    Ahead,
    Behind,
    /// Sem prazo: só progresso.
    Free,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GoalStatus {
    pub progress: f64,
    pub missing: f64,
    pub months_left: Option<i64>,
    /// Quanto guardar por mês daqui até o prazo.
    pub per_month: f64,
    /// Quanto já deveria estar guardado hoje (com prazo).
    pub expected: Option<f64>,
    pub pace: Pace,
}

/// Progresso e ritmo: compara o guardado com o esperado se o alvo fosse
/// distribuído por igual entre o início e o prazo.
pub fn goal_status(g: &Goal, today_month: &str) -> GoalStatus {
    let progress = if g.target > 0.0 { (g.saved / g.target).min(1.0) } else { 0.0 };
    let missing = (g.target - g.saved).max(0.0);
    let months_left = g.deadline.as_deref().and_then(|d| months_between(today_month, d)).map(|m| m.max(0));
    let per_month = match months_left {
        Some(m) if m > 0 => missing / m as f64,
        _ => missing,
    };
    let mut expected = None;
    let pace = if g.state == GoalState::Done || (g.target > 0.0 && g.saved >= g.target) {
        Pace::Done
    } else if g.state == GoalState::Paused {
        Pace::Paused
    } else if g.state == GoalState::Archived {
        Pace::Archived
    } else {
        match g.deadline.as_deref().and_then(|d| months_between(&g.start, d)) {
            Some(total) if total > 0 => {
                let elapsed = months_between(&g.start, today_month).unwrap_or(0);
                let e = g.target * (elapsed as f64 / total as f64).clamp(0.0, 1.0);
                expected = Some(e);
                if g.saved >= e {
                    Pace::Ahead
                } else {
                    Pace::Behind
                }
            }
            _ => Pace::Free,
        }
    };
    GoalStatus { progress, missing, months_left, per_month, expected, pace }
}

/// Totais do mês (sem transferências e sem os "fora dos totais").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MonthTotals {
    pub income_total: f64,
    pub income_received: f64,
    pub expense_total: f64,
    pub expense_paid: f64,
}

impl MonthTotals {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Self {
        let mut t = MonthTotals::default();
        for e in entries.into_iter().filter(|e| e.counts()) {
            match e.kind {
                Kind::Income => {
                    t.income_total += e.amount;
                    if e.paid {
                        t.income_received += e.amount;
                    }
                }
                Kind::Expense => {
                    t.expense_total += e.amount;
                    if e.paid {
                        t.expense_paid += e.amount;
                    }
                }
            }
        }
        t
    }
    /// Recebido − pago.
    pub fn balance(&self) -> f64 {
        self.income_received - self.expense_paid
    }
    /// Receitas − despesas do mês inteiro (inclui o que ainda vai entrar/sair).
    pub fn available(&self) -> f64 {
        self.income_total - self.expense_total
    }
    pub fn income_pending(&self) -> f64 {
        self.income_total - self.income_received
    }
    pub fn expense_pending(&self) -> f64 {
        self.expense_total - self.expense_paid
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Overview {
    pub month: String,
    pub totals: MonthTotals,
    pub previous: MonthTotals,
    pub accounts: Vec<AccountBalance>,
    pub accounts_total: f64,
    /// Saldo das contas + o que ainda vai entrar − o que ainda vai sair até o
    /// fim do mês (inclui o que ficou em aberto de meses anteriores).
    pub forecast_end: f64,
    pub days_left: u32,
    /// Quanto dá para gastar por dia até o fim do mês.
    pub daily_allowance: f64,
    /// Despesas por categoria (compras do cartão no lugar da fatura), maior primeiro.
    pub by_category: Vec<(Option<Category>, f64)>,
    /// Despesas não pagas vencidas ou vencendo nos próximos 7 dias.
    pub due_soon: Vec<Entry>,
    pub future_installments: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrendPoint {
    pub month: String,
    pub income: f64,
    pub expense: f64,
}

// ------------------------------------------------------------------- Banco

pub(crate) const MIGRATION_V6: &str = r#"
CREATE TABLE fin_account (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT    NOT NULL UNIQUE,
    initial_balance REAL    NOT NULL DEFAULT 0,
    color           INTEGER NOT NULL DEFAULT 1003,
    position        INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE TABLE fin_category (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    name  TEXT    NOT NULL,
    kind  INTEGER NOT NULL,
    color INTEGER NOT NULL DEFAULT 1021,
    UNIQUE (name, kind)
);

CREATE TABLE fin_card (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL,
    credit_limit REAL    NOT NULL DEFAULT 0,
    closing_day  INTEGER NOT NULL DEFAULT 3,
    due_day      INTEGER NOT NULL DEFAULT 10,
    account_id   INTEGER REFERENCES fin_account(id) ON DELETE SET NULL,
    color        INTEGER NOT NULL DEFAULT 1005,
    created_at   INTEGER NOT NULL
);

CREATE TABLE fin_entry (
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
CREATE INDEX ix_fin_entry_month ON fin_entry (month, kind);
CREATE UNIQUE INDEX ux_fin_entry_recurrence ON fin_entry (recurrence_group, month)
    WHERE recurrence_group IS NOT NULL;
CREATE UNIQUE INDEX ux_fin_entry_invoice ON fin_entry (card_id, month) WHERE card_id IS NOT NULL;

CREATE TABLE fin_recurrence_skip (
    recurrence_group TEXT NOT NULL,
    month            TEXT NOT NULL,
    PRIMARY KEY (recurrence_group, month)
);

CREATE TABLE fin_budget (
    category_id   INTEGER PRIMARY KEY REFERENCES fin_category(id) ON DELETE CASCADE,
    monthly_limit REAL NOT NULL
);

CREATE TABLE fin_card_purchase (
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
CREATE INDEX ix_fin_purchase_invoice ON fin_card_purchase (card_id, invoice_month);

CREATE TABLE fin_goal (
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

CREATE TABLE fin_goal_deposit (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    goal_id    INTEGER NOT NULL REFERENCES fin_goal(id) ON DELETE CASCADE,
    amount     REAL    NOT NULL,
    date       TEXT    NOT NULL,
    note       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE INDEX ix_fin_deposit_goal ON fin_goal_deposit (goal_id, date);

INSERT INTO fin_category (name, kind, color) VALUES
    ('Salário', 0, 1002), ('Vale/Benefícios', 0, 1008), ('Freelance', 0, 1003),
    ('Pix recebido', 0, 1007), ('Vendas', 0, 1015), ('Rendimentos', 0, 1016), ('Outros', 0, 1021),
    ('Mercado', 1, 1010), ('Alimentação', 1, 1001), ('Moradia', 1, 1011), ('Energia', 1, 1009),
    ('Água', 1, 1007), ('Internet', 1, 1006), ('Transporte', 1, 1012), ('Combustível', 1, 1013),
    ('Saúde', 1, 1004), ('Educação', 1, 1015), ('Lazer', 1, 1005), ('Compras', 1, 1017),
    ('Assinaturas', 1, 1020), ('Outros', 1, 1021);
"#;

const ENTRY_COLS: &str = "id, month, kind, description, amount, due_date, category_id, account_id, paid, paid_date, \
     note, excluded, recurrence_group, recurrence_until, installment_group, installment_num, installment_total, \
     transfer_group, card_id, created_at";

fn entry_from_row(r: &Row) -> Result<Entry> {
    Ok(Entry {
        id: r.get("id")?,
        month: r.get("month")?,
        kind: Kind::from_code(r.get("kind")?),
        description: r.get("description")?,
        amount: r.get("amount")?,
        due_date: r.get("due_date")?,
        category_id: r.get("category_id")?,
        account_id: r.get("account_id")?,
        paid: r.get("paid")?,
        paid_date: r.get("paid_date")?,
        note: r.get("note")?,
        excluded: r.get("excluded")?,
        recurrence_group: r.get("recurrence_group")?,
        recurrence_until: r.get("recurrence_until")?,
        installment_group: r.get("installment_group")?,
        installment_num: r.get("installment_num")?,
        installment_total: r.get("installment_total")?,
        transfer_group: r.get("transfer_group")?,
        card_id: r.get("card_id")?,
        created_at: r.get("created_at")?,
    })
}

fn account_from_row(r: &Row) -> Result<Account> {
    Ok(Account {
        id: r.get("id")?,
        name: r.get("name")?,
        initial_balance: r.get("initial_balance")?,
        color: r.get("color")?,
        position: r.get("position")?,
    })
}

fn category_from_row(r: &Row) -> Result<Category> {
    Ok(Category { id: r.get("id")?, name: r.get("name")?, kind: Kind::from_code(r.get("kind")?), color: r.get("color")? })
}

fn card_from_row(r: &Row) -> Result<Card> {
    Ok(Card {
        id: r.get("id")?,
        name: r.get("name")?,
        credit_limit: r.get("credit_limit")?,
        closing_day: r.get::<_, i64>("closing_day")?.clamp(1, 31) as u32,
        due_day: r.get::<_, i64>("due_day")?.clamp(1, 31) as u32,
        account_id: r.get("account_id")?,
        color: r.get("color")?,
    })
}

fn purchase_from_row(r: &Row) -> Result<CardPurchase> {
    Ok(CardPurchase {
        id: r.get("id")?,
        card_id: r.get("card_id")?,
        description: r.get("description")?,
        amount: r.get("amount")?,
        date: r.get("date")?,
        invoice_month: r.get("invoice_month")?,
        category_id: r.get("category_id")?,
        installment_group: r.get("installment_group")?,
        installment_num: r.get("installment_num")?,
        installment_total: r.get("installment_total")?,
    })
}

fn goal_from_row(r: &Row) -> Result<Goal> {
    Ok(Goal {
        id: r.get("id")?,
        name: r.get("name")?,
        target: r.get("target")?,
        saved: r.get("saved")?,
        deadline: r.get("deadline")?,
        start: r.get("start")?,
        state: GoalState::from_code(r.get("state")?),
        done_at: r.get("done_at")?,
        icon: r.get("icon")?,
        color: r.get("color")?,
    })
}

fn clean(text: &Option<String>) -> Option<String> {
    text.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn today_key() -> String {
    crate::checkin::day_key(crate::checkin::today())
}

impl Db {
    // ------------------------------------------------------------ Contas

    pub fn fin_accounts(&self) -> Result<Vec<Account>> {
        let mut st = self.conn.prepare("SELECT * FROM fin_account ORDER BY position, name")?;
        let rows = st.query_map([], account_from_row)?;
        rows.collect()
    }

    pub fn fin_insert_account(&self, name: &str, initial_balance: f64, color: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fin_account (name, initial_balance, color, position, created_at) VALUES (?1, ?2, ?3, \
             (SELECT COALESCE(MAX(position), -1) + 1 FROM fin_account), ?4)",
            params![name.trim(), initial_balance, color, crate::now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn fin_update_account(&self, a: &Account) -> Result<()> {
        self.conn.execute(
            "UPDATE fin_account SET name = ?1, initial_balance = ?2, color = ?3 WHERE id = ?4",
            params![a.name.trim(), a.initial_balance, a.color, a.id],
        )?;
        Ok(())
    }

    /// Os lançamentos ficam, sem conta.
    pub fn fin_delete_account(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM fin_account WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Saldo atual e previsto de cada conta, com receitas/despesas de `month`.
    pub fn fin_account_balances(&self, month: &str) -> Result<Vec<AccountBalance>> {
        let accounts = self.fin_accounts()?;
        let mut st = self.conn.prepare(
            "SELECT account_id, \
               COALESCE(SUM(CASE WHEN paid = 1 THEN (CASE kind WHEN 0 THEN amount ELSE -amount END) END), 0), \
               COALESCE(SUM(CASE WHEN paid = 0 AND month <= ?1 THEN (CASE kind WHEN 0 THEN amount ELSE -amount END) END), 0), \
               COALESCE(SUM(CASE WHEN month = ?1 AND kind = 0 AND transfer_group IS NULL THEN amount END), 0), \
               COALESCE(SUM(CASE WHEN month = ?1 AND kind = 1 AND transfer_group IS NULL THEN amount END), 0) \
             FROM fin_entry WHERE account_id IS NOT NULL GROUP BY account_id",
        )?;
        let sums: HashMap<i64, (f64, f64, f64, f64)> = st
            .query_map([month], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
            .collect::<Result<_>>()?;
        Ok(accounts
            .into_iter()
            .map(|a| {
                let (paid, pending, inc, exp) = sums.get(&a.id).copied().unwrap_or_default();
                let current = a.initial_balance + paid;
                AccountBalance { account: a, current, forecast: current + pending, month_income: inc, month_expense: exp }
            })
            .collect())
    }

    // -------------------------------------------------------- Categorias

    pub fn fin_categories(&self) -> Result<Vec<Category>> {
        let mut st = self.conn.prepare("SELECT * FROM fin_category")?;
        let mut cats: Vec<Category> = st.query_map([], category_from_row)?.collect::<Result<_>>()?;
        cats.sort_by_cached_key(|c| (c.kind.code(), sort_key(&c.name)));
        Ok(cats)
    }

    pub fn fin_insert_category(&self, name: &str, kind: Kind, color: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fin_category (name, kind, color) VALUES (?1, ?2, ?3)",
            params![name.trim(), kind.code(), color],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn fin_update_category(&self, c: &Category) -> Result<()> {
        self.conn.execute(
            "UPDATE fin_category SET name = ?1, color = ?2 WHERE id = ?3",
            params![c.name.trim(), c.color, c.id],
        )?;
        Ok(())
    }

    /// Lançamentos e compras ficam sem categoria; o orçamento dela some.
    pub fn fin_delete_category(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM fin_category WHERE id = ?1", [id])?;
        Ok(())
    }

    // ------------------------------------------------------- Lançamentos

    fn fin_query_entries(&self, sql_where: &str, p: impl rusqlite::Params) -> Result<Vec<Entry>> {
        let mut st = self.conn.prepare(&format!("SELECT {ENTRY_COLS} FROM fin_entry WHERE {sql_where}"))?;
        let rows = st.query_map(p, entry_from_row)?;
        rows.collect()
    }

    pub fn fin_entry(&self, id: i64) -> Result<Option<Entry>> {
        self.conn
            .query_row(&format!("SELECT {ENTRY_COLS} FROM fin_entry WHERE id = ?1"), [id], entry_from_row)
            .optional()
    }

    /// Lançamentos do mês (cria antes as ocorrências das séries recorrentes).
    pub fn fin_entries(&self, month: &str) -> Result<Vec<Entry>> {
        self.fin_ensure_recurring(month)?;
        self.fin_query_entries("month = ?1 ORDER BY COALESCE(due_date, paid_date, '9999'), id", [month])
    }

    /// Despesas de meses anteriores que ainda não foram pagas. De cada série
    /// recorrente só aparece a ocorrência em aberto mais antiga.
    pub fn fin_overdue_before(&self, month: &str) -> Result<Vec<Entry>> {
        self.fin_query_entries(
            "month < ?1 AND kind = 1 AND paid = 0 AND transfer_group IS NULL AND (recurrence_group IS NULL OR \
             month = (SELECT MIN(e2.month) FROM fin_entry e2 WHERE e2.recurrence_group = fin_entry.recurrence_group \
             AND e2.paid = 0)) ORDER BY month, COALESCE(due_date, '9999'), id",
            [month],
        )
    }

    /// Cria no mês as ocorrências que faltam das séries recorrentes. Devolve quantas criou.
    pub fn fin_ensure_recurring(&self, month: &str) -> Result<usize> {
        let groups: Vec<String> = {
            let mut st = self.conn.prepare(
                "SELECT recurrence_group FROM fin_entry WHERE recurrence_group IS NOT NULL \
                 GROUP BY recurrence_group HAVING MIN(month) < ?1 \
                 AND SUM(month = ?1) = 0 \
                 AND NOT EXISTS (SELECT 1 FROM fin_recurrence_skip s WHERE s.recurrence_group = fin_entry.recurrence_group AND s.month = ?1)",
            )?;
            let rows = st.query_map([month], |r| r.get(0))?;
            rows.collect::<Result<_>>()?
        };
        if groups.is_empty() {
            return Ok(0);
        }
        self.transaction(|| {
            let mut created = 0;
            for group in groups {
                let model = self
                    .fin_query_entries(
                        "recurrence_group = ?1 AND month < ?2 ORDER BY month DESC LIMIT 1",
                        params![group, month],
                    )?
                    .pop();
                let Some(mut e) = model else { continue };
                if e.recurrence_until.as_deref().is_some_and(|until| month > until) {
                    continue;
                }
                e.due_date = e.due_date.as_deref().and_then(parse_date).map(|d| day_in_month(month, d.day()));
                e.month = month.to_string();
                e.paid = false;
                e.paid_date = None;
                self.fin_insert_row(&e)?;
                created += 1;
            }
            Ok(created)
        })
    }

    fn fin_insert_row(&self, e: &Entry) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fin_entry (month, kind, description, amount, due_date, category_id, account_id, paid, \
             paid_date, note, excluded, recurrence_group, recurrence_until, installment_group, installment_num, \
             installment_total, transfer_group, card_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                e.month,
                e.kind.code(),
                e.description.trim(),
                e.amount,
                clean(&e.due_date),
                e.category_id,
                e.account_id,
                e.paid,
                if e.paid { clean(&e.paid_date).or_else(|| Some(today_key())) } else { None },
                e.note.trim(),
                e.excluded,
                e.recurrence_group,
                e.recurrence_until,
                e.installment_group,
                e.installment_num,
                e.installment_total,
                e.transfer_group,
                e.card_id,
                if e.created_at == 0 { crate::now_ms() } else { e.created_at },
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn fin_set_group(&self, column: &str, id: i64, group: &str) -> Result<()> {
        self.conn.execute(&format!("UPDATE fin_entry SET {column} = ?1 WHERE id = ?2"), params![group, id])?;
        Ok(())
    }

    /// Cria um lançamento (série recorrente ou parcelas conforme `repeat`).
    /// Devolve o id do primeiro.
    pub fn fin_add_entry(&self, e: &Entry, repeat: Repeat) -> Result<i64> {
        self.transaction(|| match repeat {
            Repeat::Once => {
                let mut e = e.clone();
                e.recurrence_group = None;
                e.recurrence_until = None;
                self.fin_insert_row(&e)
            }
            Repeat::Monthly(times) => {
                let mut e = e.clone();
                e.recurrence_until = times.filter(|n| *n > 0).map(|n| add_months(&e.month, n as i32 - 1));
                let id = self.fin_insert_row(&e)?;
                self.fin_set_group("recurrence_group", id, &format!("r{id}"))?;
                Ok(id)
            }
            Repeat::Installments(n) => {
                let n = n.max(1);
                let day = e.due_date.as_deref().and_then(parse_date).map(|d| d.day());
                let mut first = 0;
                for i in 0..n {
                    let mut p = e.clone();
                    p.month = add_months(&e.month, i as i32);
                    p.due_date = day.map(|d| day_in_month(&p.month, d));
                    p.description = if n > 1 { format!("{} ({}/{n})", e.description.trim(), i + 1) } else { e.description.clone() };
                    p.installment_num = (n > 1).then_some(i64::from(i) + 1);
                    p.installment_total = (n > 1).then_some(i64::from(n));
                    p.installment_group = (n > 1 && i > 0).then(|| format!("p{first}"));
                    p.recurrence_group = None;
                    p.recurrence_until = None;
                    if i > 0 {
                        p.paid = false;
                        p.paid_date = None;
                    }
                    let id = self.fin_insert_row(&p)?;
                    if i == 0 {
                        first = id;
                        if n > 1 {
                            self.fin_set_group("installment_group", id, &format!("p{id}"))?;
                        }
                    }
                }
                Ok(first)
            }
        })
    }

    /// Salva as alterações. Com `Scope::Following`, descrição, valor, categoria,
    /// conta, dia do vencimento, observação e "fora dos totais" valem também
    /// para as próximas ocorrências da série (ou parcelas seguintes).
    pub fn fin_update_entry(&self, e: &Entry, scope: Scope) -> Result<()> {
        self.transaction(|| {
            self.conn.execute(
                "UPDATE fin_entry SET month = ?1, kind = ?2, description = ?3, amount = ?4, due_date = ?5, \
                 category_id = ?6, account_id = ?7, paid = ?8, paid_date = ?9, note = ?10, excluded = ?11 WHERE id = ?12",
                params![
                    e.month,
                    e.kind.code(),
                    e.description.trim(),
                    e.amount,
                    clean(&e.due_date),
                    e.category_id,
                    e.account_id,
                    e.paid,
                    if e.paid { clean(&e.paid_date).or_else(|| Some(today_key())) } else { None },
                    e.note.trim(),
                    e.excluded,
                    e.id,
                ],
            )?;
            if scope == Scope::This {
                return Ok(());
            }
            let day = e.due_date.as_deref().and_then(parse_date).map(|d| d.day());
            let followers = if let Some(group) = &e.recurrence_group {
                self.fin_query_entries("recurrence_group = ?1 AND month > ?2", params![group, e.month])?
            } else if let Some(group) = &e.installment_group {
                self.fin_query_entries("installment_group = ?1 AND month > ?2", params![group, e.month])?
            } else {
                Vec::new()
            };
            let base = strip_installment_suffix(&e.description);
            for f in followers {
                let description = match (f.installment_num, f.installment_total) {
                    (Some(i), Some(n)) if e.is_installment() => format!("{base} ({i}/{n})"),
                    _ => e.description.trim().to_string(),
                };
                self.conn.execute(
                    "UPDATE fin_entry SET description = ?1, amount = ?2, due_date = ?3, category_id = ?4, \
                     account_id = ?5, note = ?6, excluded = ?7 WHERE id = ?8",
                    params![
                        description,
                        e.amount,
                        day.map(|d| day_in_month(&f.month, d)),
                        e.category_id,
                        e.account_id,
                        e.note.trim(),
                        e.excluded,
                        f.id
                    ],
                )?;
            }
            Ok(())
        })
    }

    pub fn fin_set_paid(&self, id: i64, paid: bool, date: Option<&str>) -> Result<()> {
        let date = if paid { Some(date.map_or_else(today_key, str::to_string)) } else { None };
        self.conn.execute("UPDATE fin_entry SET paid = ?1, paid_date = ?2 WHERE id = ?3", params![paid, date, id])?;
        Ok(())
    }

    pub fn fin_set_excluded(&self, id: i64, excluded: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE fin_entry SET excluded = ?1 WHERE id = ?2 AND transfer_group IS NULL",
            params![excluded, id],
        )?;
        Ok(())
    }

    /// Para a série recorrente em `last_month`: apaga as ocorrências seguintes
    /// que ainda não foram pagas e não gera mais nenhuma.
    pub fn fin_end_recurrence(&self, group: &str, last_month: &str) -> Result<()> {
        self.transaction(|| {
            self.conn.execute(
                "DELETE FROM fin_entry WHERE recurrence_group = ?1 AND month > ?2 AND paid = 0",
                params![group, last_month],
            )?;
            self.conn.execute(
                "UPDATE fin_entry SET recurrence_until = ?2 WHERE recurrence_group = ?1",
                params![group, last_month],
            )?;
            Ok(())
        })
    }

    /// Apaga o lançamento. Transferência apaga as duas pernas. Numa série,
    /// `Scope` decide se vai só este, este e os próximos, ou todos.
    /// Lançamento de fatura não é apagado aqui (apague as compras do cartão).
    pub fn fin_delete_entry(&self, id: i64, scope: Scope) -> Result<()> {
        let Some(e) = self.fin_entry(id)? else { return Ok(()) };
        if e.is_invoice() {
            return Ok(());
        }
        self.transaction(|| {
            if let Some(group) = &e.transfer_group {
                self.conn.execute("DELETE FROM fin_entry WHERE transfer_group = ?1", [group])?;
                return Ok(());
            }
            if let Some(group) = &e.recurrence_group {
                match scope {
                    Scope::All => {
                        self.conn.execute("DELETE FROM fin_entry WHERE recurrence_group = ?1", [group])?;
                        self.conn.execute("DELETE FROM fin_recurrence_skip WHERE recurrence_group = ?1", [group])?;
                    }
                    Scope::Following => {
                        self.conn.execute("DELETE FROM fin_entry WHERE id = ?1", [id])?;
                        self.fin_end_recurrence(group, &add_months(&e.month, -1))?;
                    }
                    Scope::This => {
                        self.conn.execute("DELETE FROM fin_entry WHERE id = ?1", [id])?;
                        // Sem o pulo, a ocorrência voltaria ao abrir o mês de novo.
                        self.conn.execute(
                            "INSERT OR IGNORE INTO fin_recurrence_skip (recurrence_group, month) VALUES (?1, ?2)",
                            params![group, e.month],
                        )?;
                    }
                }
                return Ok(());
            }
            if let Some(group) = &e.installment_group {
                match scope {
                    Scope::All => {
                        self.conn.execute("DELETE FROM fin_entry WHERE installment_group = ?1", [group])?;
                        return Ok(());
                    }
                    Scope::Following => {
                        self.conn.execute(
                            "DELETE FROM fin_entry WHERE installment_group = ?1 AND month >= ?2",
                            params![group, e.month],
                        )?;
                        return Ok(());
                    }
                    Scope::This => {}
                }
            }
            self.conn.execute("DELETE FROM fin_entry WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// Transferência entre contas: saída paga na origem, entrada paga no destino.
    pub fn fin_transfer(&self, from: i64, to: i64, amount: f64, date: &str, description: &str) -> Result<i64> {
        let accounts = self.fin_accounts()?;
        let name = |id: i64| accounts.iter().find(|a| a.id == id).map_or_else(String::new, |a| a.name.clone());
        let (from_name, to_name) = (name(from), name(to));
        let base = if description.trim().is_empty() { "Transferência" } else { description.trim() };
        let month = parse_date(date).map_or_else(current_month, month_of);
        self.transaction(|| {
            let mut out = Entry::new(Kind::Expense, &format!("{base} → {to_name}"), amount, &month);
            out.account_id = Some(from);
            out.due_date = Some(date.to_string());
            out.paid = true;
            out.paid_date = Some(date.to_string());
            let out_id = self.fin_insert_row(&out)?;
            let group = format!("t{out_id}");
            self.fin_set_group("transfer_group", out_id, &group)?;
            let mut inc = Entry::new(Kind::Income, &format!("{base} de {from_name}"), amount, &month);
            inc.account_id = Some(to);
            inc.due_date = Some(date.to_string());
            inc.paid = true;
            inc.paid_date = Some(date.to_string());
            inc.transfer_group = Some(group);
            self.fin_insert_row(&inc)?;
            Ok(out_id)
        })
    }

    /// Totais de um mês (garante as recorrências antes).
    pub fn fin_month_totals(&self, month: &str) -> Result<MonthTotals> {
        Ok(MonthTotals::from_entries(&self.fin_entries(month)?))
    }

    /// Receitas e despesas dos `n` meses terminando em `last`.
    pub fn fin_trend(&self, last: &str, n: u32) -> Result<Vec<TrendPoint>> {
        (0..n as i32)
            .rev()
            .map(|i| {
                let month = add_months(last, -i);
                let t = self.fin_month_totals(&month)?;
                Ok(TrendPoint { month, income: t.income_total, expense: t.expense_total })
            })
            .collect()
    }

    /// Despesas do mês por categoria: lançamentos (menos as faturas) + compras
    /// dos cartões cuja fatura é deste mês.
    pub fn fin_spending_by_category(&self, month: &str) -> Result<HashMap<Option<i64>, f64>> {
        let mut out: HashMap<Option<i64>, f64> = HashMap::new();
        let mut st = self.conn.prepare(
            "SELECT category_id, SUM(amount) FROM fin_entry WHERE month = ?1 AND kind = 1 AND excluded = 0 \
             AND transfer_group IS NULL AND card_id IS NULL GROUP BY category_id \
             UNION ALL \
             SELECT p.category_id, SUM(p.amount) FROM fin_card_purchase p WHERE p.invoice_month = ?1 \
             AND NOT EXISTS (SELECT 1 FROM fin_entry e WHERE e.card_id = p.card_id AND e.month = ?1 AND e.excluded = 1) \
             GROUP BY p.category_id",
        )?;
        let rows = st.query_map([month], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, f64>(1)?)))?;
        for row in rows {
            let (cat, total) = row?;
            *out.entry(cat).or_default() += total;
        }
        Ok(out)
    }

    /// Resumo do mês para o painel.
    pub fn fin_overview(&self, month: &str) -> Result<Overview> {
        let today = crate::checkin::today();
        let entries = self.fin_entries(month)?;
        let totals = MonthTotals::from_entries(&entries);
        let previous = self.fin_month_totals(&add_months(month, -1))?;
        let accounts = self.fin_account_balances(month)?;
        let accounts_total: f64 = accounts.iter().map(|a| a.current).sum();
        let this_month = current_month();
        let days_left = match month.cmp(this_month.as_str()) {
            std::cmp::Ordering::Equal => days_in_month(month) - today.day() + 1,
            std::cmp::Ordering::Greater => days_in_month(month),
            std::cmp::Ordering::Less => 0,
        };
        let daily_allowance =
            if days_left > 0 { (totals.available().max(0.0) / f64::from(days_left) * 100.0).round() / 100.0 } else { 0.0 };

        let categories = self.fin_categories()?;
        let mut by_category: Vec<(Option<Category>, f64)> = self
            .fin_spending_by_category(month)?
            .into_iter()
            .filter(|(_, v)| *v > 0.0)
            .map(|(id, v)| (id.and_then(|id| categories.iter().find(|c| c.id == id).cloned()), v))
            .collect();
        by_category.sort_by(|a, b| b.1.total_cmp(&a.1));

        let limit = crate::checkin::day_key(today.checked_add_days(Days::new(7)).unwrap_or(today));
        let due_soon = self.fin_query_entries(
            "kind = 1 AND paid = 0 AND due_date IS NOT NULL AND due_date <= ?1 ORDER BY due_date, id",
            [limit],
        )?;
        let pending: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(CASE kind WHEN 0 THEN amount ELSE -amount END), 0) FROM fin_entry \
             WHERE paid = 0 AND month <= ?1",
            [month],
            |r| r.get(0),
        )?;
        let future_installments = self.fin_query_entries(
            "installment_total > 1 AND month > ?1 ORDER BY month, id LIMIT 20",
            [month],
        )?;
        Ok(Overview {
            month: month.to_string(),
            totals,
            previous,
            forecast_end: accounts_total + pending,
            accounts,
            accounts_total,
            days_left,
            daily_allowance,
            by_category,
            due_soon,
            future_installments,
        })
    }

    // ----------------------------------------------------------- Orçamento

    pub fn fin_budget(&self, month: &str) -> Result<Vec<BudgetLine>> {
        let categories = self.fin_categories()?;
        let spent = self.fin_spending_by_category(month)?;
        let mut st = self.conn.prepare("SELECT category_id, monthly_limit FROM fin_budget")?;
        let limits: Vec<(i64, f64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_>>()?;
        let mut lines: Vec<BudgetLine> = limits
            .into_iter()
            .filter_map(|(id, limit)| {
                let category = categories.iter().find(|c| c.id == id)?.clone();
                Some(BudgetLine { spent: spent.get(&Some(id)).copied().unwrap_or(0.0), category, limit })
            })
            .collect();
        lines.sort_by(|a, b| b.fraction().total_cmp(&a.fraction()));
        Ok(lines)
    }

    /// Define o limite mensal da categoria (zero ou menos remove).
    pub fn fin_set_budget(&self, category_id: i64, limit: f64) -> Result<()> {
        if limit <= 0.0 {
            self.conn.execute("DELETE FROM fin_budget WHERE category_id = ?1", [category_id])?;
        } else {
            self.conn.execute(
                "INSERT INTO fin_budget (category_id, monthly_limit) VALUES (?1, ?2) \
                 ON CONFLICT(category_id) DO UPDATE SET monthly_limit = excluded.monthly_limit",
                params![category_id, limit],
            )?;
        }
        Ok(())
    }

    // ------------------------------------------------------------- Cartões

    pub fn fin_cards(&self) -> Result<Vec<Card>> {
        let mut st = self.conn.prepare("SELECT * FROM fin_card ORDER BY id")?;
        let rows = st.query_map([], card_from_row)?;
        rows.collect()
    }

    pub fn fin_card(&self, id: i64) -> Result<Option<Card>> {
        self.conn.query_row("SELECT * FROM fin_card WHERE id = ?1", [id], card_from_row).optional()
    }

    pub fn fin_insert_card(&self, c: &Card) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fin_card (name, credit_limit, closing_day, due_day, account_id, color, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![c.name.trim(), c.credit_limit, c.closing_day, c.due_day, c.account_id, c.color, crate::now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Atualiza o cartão e as faturas em aberto (nome, conta e vencimento).
    pub fn fin_update_card(&self, c: &Card) -> Result<()> {
        self.transaction(|| {
            self.conn.execute(
                "UPDATE fin_card SET name = ?1, credit_limit = ?2, closing_day = ?3, due_day = ?4, account_id = ?5, \
                 color = ?6 WHERE id = ?7",
                params![c.name.trim(), c.credit_limit, c.closing_day, c.due_day, c.account_id, c.color, c.id],
            )?;
            let months: Vec<String> = {
                let mut st = self.conn.prepare("SELECT month FROM fin_entry WHERE card_id = ?1 AND paid = 0")?;
                let rows = st.query_map([c.id], |r| r.get(0))?;
                rows.collect::<Result<_>>()?
            };
            for m in months {
                self.fin_sync_invoice(c.id, &m)?;
            }
            Ok(())
        })
    }

    /// Apaga o cartão e as compras. Faturas já pagas ficam como despesas comuns.
    pub fn fin_delete_card(&self, id: i64) -> Result<()> {
        self.transaction(|| {
            self.conn.execute("DELETE FROM fin_entry WHERE card_id = ?1 AND paid = 0", [id])?;
            self.conn.execute("DELETE FROM fin_card WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    pub fn fin_invoice_entry(&self, card_id: i64, month: &str) -> Result<Option<Entry>> {
        Ok(self.fin_query_entries("card_id = ?1 AND month = ?2", params![card_id, month])?.pop())
    }

    /// Mantém o lançamento da fatura igual à soma das compras do mês.
    pub fn fin_sync_invoice(&self, card_id: i64, month: &str) -> Result<()> {
        let Some(card) = self.fin_card(card_id)? else { return Ok(()) };
        let total: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM fin_card_purchase WHERE card_id = ?1 AND invoice_month = ?2",
            params![card_id, month],
            |r| r.get(0),
        )?;
        let total = (total * 100.0).round() / 100.0;
        let existing = self.fin_invoice_entry(card_id, month)?;
        let description = format!("Fatura {}", card.name);
        let due = day_in_month(month, card.due_day);
        match existing {
            Some(e) if total.abs() < 0.005 && !e.paid => {
                self.conn.execute("DELETE FROM fin_entry WHERE id = ?1", [e.id])?;
            }
            Some(e) => {
                self.conn.execute(
                    "UPDATE fin_entry SET amount = ?1, description = ?2, due_date = ?3, account_id = ?4 WHERE id = ?5",
                    params![total, description, due, card.account_id, e.id],
                )?;
            }
            None if total.abs() >= 0.005 => {
                let mut e = Entry::new(Kind::Expense, &description, total, month);
                e.due_date = Some(due);
                e.account_id = card.account_id;
                e.card_id = Some(card_id);
                self.fin_insert_row(&e)?;
            }
            None => {}
        }
        Ok(())
    }

    /// Compras da fatura de `month`.
    pub fn fin_purchases(&self, card_id: i64, month: &str) -> Result<Vec<CardPurchase>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM fin_card_purchase WHERE card_id = ?1 AND invoice_month = ?2 ORDER BY date DESC, id DESC",
        )?;
        let rows = st.query_map(params![card_id, month], purchase_from_row)?;
        rows.collect()
    }

    /// Registra uma compra de `total` em `installments` parcelas (uma por fatura).
    pub fn fin_add_purchase(
        &self,
        card_id: i64,
        description: &str,
        total: f64,
        date: &str,
        category_id: Option<i64>,
        installments: u32,
    ) -> Result<()> {
        let Some(card) = self.fin_card(card_id)? else { return Ok(()) };
        let day = parse_date(date).unwrap_or_else(crate::checkin::today);
        let first_invoice = card.invoice_for(day);
        let parts = split_installments(total, installments);
        let n = parts.len() as i64;
        self.transaction(|| {
            let mut group: Option<String> = None;
            for (i, amount) in parts.iter().enumerate() {
                let invoice = add_months(&first_invoice, i as i32);
                let desc = if n > 1 { format!("{} ({}/{n})", description.trim(), i + 1) } else { description.trim().to_string() };
                self.conn.execute(
                    "INSERT INTO fin_card_purchase (card_id, description, amount, date, invoice_month, category_id, \
                     installment_group, installment_num, installment_total, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        card_id,
                        desc,
                        amount,
                        crate::checkin::day_key(day),
                        invoice,
                        category_id,
                        group,
                        (n > 1).then_some(i as i64 + 1),
                        (n > 1).then_some(n),
                        crate::now_ms()
                    ],
                )?;
                if n > 1 && i == 0 {
                    let id = self.conn.last_insert_rowid();
                    let g = format!("c{id}");
                    self.conn.execute("UPDATE fin_card_purchase SET installment_group = ?1 WHERE id = ?2", params![g, id])?;
                    group = Some(g);
                }
                self.fin_sync_invoice(card_id, &invoice)?;
            }
            Ok(())
        })
    }

    /// Apaga a compra (com `all_installments`, todas as parcelas dela).
    pub fn fin_delete_purchase(&self, id: i64, all_installments: bool) -> Result<()> {
        let p = self.conn.query_row("SELECT * FROM fin_card_purchase WHERE id = ?1", [id], purchase_from_row).optional()?;
        let Some(p) = p else { return Ok(()) };
        self.transaction(|| {
            let affected: Vec<String> = match (&p.installment_group, all_installments) {
                (Some(group), true) => {
                    let months: Vec<String> = {
                        let mut st =
                            self.conn.prepare("SELECT DISTINCT invoice_month FROM fin_card_purchase WHERE installment_group = ?1")?;
                        let rows = st.query_map([group], |r| r.get(0))?;
                        rows.collect::<Result<_>>()?
                    };
                    self.conn.execute("DELETE FROM fin_card_purchase WHERE installment_group = ?1", [group])?;
                    months
                }
                _ => {
                    self.conn.execute("DELETE FROM fin_card_purchase WHERE id = ?1", [id])?;
                    vec![p.invoice_month.clone()]
                }
            };
            for m in affected {
                self.fin_sync_invoice(p.card_id, &m)?;
            }
            Ok(())
        })
    }

    /// Cartões com a fatura de `month` e o total em aberto.
    pub fn fin_card_summaries(&self, month: &str) -> Result<Vec<CardSummary>> {
        let cards = self.fin_cards()?;
        let mut out = Vec::new();
        for card in cards {
            let invoice = self.fin_invoice_entry(card.id, month)?;
            let open_total: f64 = self.conn.query_row(
                "SELECT COALESCE(SUM(p.amount), 0) FROM fin_card_purchase p WHERE p.card_id = ?1 AND NOT EXISTS \
                 (SELECT 1 FROM fin_entry e WHERE e.card_id = p.card_id AND e.month = p.invoice_month AND e.paid = 1)",
                [card.id],
                |r| r.get(0),
            )?;
            out.push(CardSummary {
                invoice: invoice.as_ref().map_or(0.0, |e| e.amount),
                invoice_paid: invoice.as_ref().is_some_and(|e| e.paid),
                open_total,
                card,
            });
        }
        Ok(out)
    }

    // --------------------------------------------------------------- Metas

    fn fin_goals_where(&self, sql_where: &str, p: impl rusqlite::Params) -> Result<Vec<Goal>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT g.*, COALESCE((SELECT SUM(amount) FROM fin_goal_deposit d WHERE d.goal_id = g.id), 0) AS saved \
             FROM fin_goal g WHERE {sql_where} ORDER BY g.state, g.created_at"
        ))?;
        let rows = st.query_map(p, goal_from_row)?;
        rows.collect()
    }

    pub fn fin_goals(&self) -> Result<Vec<Goal>> {
        self.fin_goals_where("1 = 1", [])
    }

    pub fn fin_goal(&self, id: i64) -> Result<Option<Goal>> {
        Ok(self.fin_goals_where("g.id = ?1", [id])?.pop())
    }

    pub fn fin_insert_goal(&self, g: &Goal) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fin_goal (name, target, deadline, start, state, done_at, icon, color, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                g.name.trim(),
                g.target,
                clean(&g.deadline),
                g.start,
                g.state.code(),
                g.done_at,
                g.icon,
                g.color,
                crate::now_ms()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn fin_update_goal(&self, g: &Goal) -> Result<()> {
        self.conn.execute(
            "UPDATE fin_goal SET name = ?1, target = ?2, deadline = ?3, icon = ?4, color = ?5 WHERE id = ?6",
            params![g.name.trim(), g.target, clean(&g.deadline), g.icon, g.color, g.id],
        )?;
        self.fin_refresh_goal_state(g.id)
    }

    pub fn fin_set_goal_state(&self, id: i64, state: GoalState) -> Result<()> {
        let done_at = (state == GoalState::Done).then(crate::now_ms);
        self.conn.execute(
            "UPDATE fin_goal SET state = ?1, done_at = ?2 WHERE id = ?3",
            params![state.code(), done_at, id],
        )?;
        Ok(())
    }

    pub fn fin_delete_goal(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM fin_goal WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn fin_deposits(&self, goal_id: i64) -> Result<Vec<GoalDeposit>> {
        let mut st =
            self.conn.prepare("SELECT * FROM fin_goal_deposit WHERE goal_id = ?1 ORDER BY date DESC, id DESC")?;
        let rows = st.query_map([goal_id], |r| {
            Ok(GoalDeposit {
                id: r.get("id")?,
                goal_id: r.get("goal_id")?,
                amount: r.get("amount")?,
                date: r.get("date")?,
                note: r.get("note")?,
            })
        })?;
        rows.collect()
    }

    /// Ativa que bate o alvo vira concluída; concluída que volta a ficar abaixo, ativa.
    fn fin_refresh_goal_state(&self, goal_id: i64) -> Result<()> {
        let Some(g) = self.fin_goal(goal_id)? else { return Ok(()) };
        let reached = g.target > 0.0 && g.saved + 0.005 >= g.target;
        match g.state {
            GoalState::Active if reached => self.fin_set_goal_state(goal_id, GoalState::Done),
            GoalState::Done if !reached => self.fin_set_goal_state(goal_id, GoalState::Active),
            _ => Ok(()),
        }
    }

    /// Depósito (negativo = retirada). Devolve o novo total guardado.
    pub fn fin_add_deposit(&self, goal_id: i64, amount: f64, date: &str, note: &str) -> Result<f64> {
        self.transaction(|| {
            self.conn.execute(
                "INSERT INTO fin_goal_deposit (goal_id, amount, date, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![goal_id, amount, date, note.trim(), crate::now_ms()],
            )?;
            self.fin_refresh_goal_state(goal_id)?;
            Ok(self.fin_goal(goal_id)?.map_or(0.0, |g| g.saved))
        })
    }

    pub fn fin_delete_deposit(&self, id: i64) -> Result<()> {
        let goal_id: Option<i64> =
            self.conn.query_row("SELECT goal_id FROM fin_goal_deposit WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        self.conn.execute("DELETE FROM fin_goal_deposit WHERE id = ?1", [id])?;
        if let Some(g) = goal_id {
            self.fin_refresh_goal_state(g)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------- Backup

    /// Apaga todos os dados financeiros (para restaurar um backup).
    pub fn fin_delete_all(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM fin_goal_deposit; DELETE FROM fin_goal; DELETE FROM fin_card_purchase; \
             DELETE FROM fin_entry; DELETE FROM fin_recurrence_skip; DELETE FROM fin_budget; DELETE FROM fin_card; \
             DELETE FROM fin_account; DELETE FROM fin_category;",
        )
    }

    /// Tabelas financeiras como JSON (linhas cruas, com ids originais).
    pub fn fin_export(&self) -> Result<Value> {
        let mut out = serde_json::Map::new();
        for table in FIN_TABLES {
            let mut st = self.conn.prepare(&format!("SELECT * FROM {table}"))?;
            let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
            let rows = st.query_map([], |r| {
                let mut obj = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    let v = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(n) => json!(n),
                        rusqlite::types::ValueRef::Real(f) => json!(f),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        rusqlite::types::ValueRef::Blob(_) => Value::Null,
                    };
                    obj.insert(name.clone(), v);
                }
                Ok(Value::Object(obj))
            })?;
            out.insert((*table).to_string(), Value::Array(rows.collect::<Result<_>>()?));
        }
        Ok(Value::Object(out))
    }

    /// Importa o que `fin_export` gerou, remapeando ids. Categorias e contas com
    /// o mesmo nome são reaproveitadas. Devolve quantos lançamentos entraram.
    pub fn fin_import(&self, data: &Value) -> Result<usize> {
        let rows = |table: &str| -> Vec<serde_json::Map<String, Value>> {
            data.get(table)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect())
                .unwrap_or_default()
        };
        let i = |o: &serde_json::Map<String, Value>, k: &str| o.get(k).and_then(Value::as_i64);
        let f = |o: &serde_json::Map<String, Value>, k: &str| o.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let s = |o: &serde_json::Map<String, Value>, k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
        let now = crate::now_ms();
        self.transaction(|| {
            let mut cat_map = HashMap::new();
            for c in rows("fin_category") {
                let (name, kind) = (s(&c, "name").unwrap_or_default(), i(&c, "kind").unwrap_or(1));
                if name.trim().is_empty() {
                    continue;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO fin_category (name, kind, color) VALUES (?1, ?2, ?3)",
                    params![name.trim(), kind, i(&c, "color").unwrap_or(1021)],
                )?;
                let id: i64 = self.conn.query_row(
                    "SELECT id FROM fin_category WHERE name = ?1 AND kind = ?2",
                    params![name.trim(), kind],
                    |r| r.get(0),
                )?;
                cat_map.insert(i(&c, "id").unwrap_or(0), id);
            }
            let mut acc_map = HashMap::new();
            for a in rows("fin_account") {
                let name = s(&a, "name").unwrap_or_default();
                if name.trim().is_empty() {
                    continue;
                }
                let existing: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM fin_account WHERE name = ?1", [name.trim()], |r| r.get(0))
                    .optional()?;
                let id = match existing {
                    Some(id) => id,
                    None => self.fin_insert_account(&name, f(&a, "initial_balance"), i(&a, "color").unwrap_or(1003))?,
                };
                acc_map.insert(i(&a, "id").unwrap_or(0), id);
            }
            let mut card_map = HashMap::new();
            for c in rows("fin_card") {
                let card = Card {
                    id: 0,
                    name: s(&c, "name").unwrap_or_else(|| "Cartão".into()),
                    credit_limit: f(&c, "credit_limit"),
                    closing_day: i(&c, "closing_day").unwrap_or(3).clamp(1, 31) as u32,
                    due_day: i(&c, "due_day").unwrap_or(10).clamp(1, 31) as u32,
                    account_id: i(&c, "account_id").and_then(|old| acc_map.get(&old).copied()),
                    color: i(&c, "color").unwrap_or(1005),
                };
                card_map.insert(i(&c, "id").unwrap_or(0), self.fin_insert_card(&card)?);
            }
            // Grupos (séries, parcelas, transferências) ganham prefixo novo para
            // não colidir com os que já existem no banco.
            let prefix = format!("i{now}-");
            let regroup = |g: Option<String>| g.map(|g| format!("{prefix}{g}"));
            let mut count = 0;
            for e in rows("fin_entry") {
                let entry = Entry {
                    id: 0,
                    month: s(&e, "month").unwrap_or_else(current_month),
                    kind: Kind::from_code(i(&e, "kind").unwrap_or(1)),
                    description: s(&e, "description").unwrap_or_default(),
                    amount: f(&e, "amount"),
                    due_date: s(&e, "due_date"),
                    category_id: i(&e, "category_id").and_then(|old| cat_map.get(&old).copied()),
                    account_id: i(&e, "account_id").and_then(|old| acc_map.get(&old).copied()),
                    paid: i(&e, "paid").unwrap_or(0) != 0,
                    paid_date: s(&e, "paid_date"),
                    note: s(&e, "note").unwrap_or_default(),
                    excluded: i(&e, "excluded").unwrap_or(0) != 0,
                    recurrence_group: regroup(s(&e, "recurrence_group")),
                    recurrence_until: s(&e, "recurrence_until"),
                    installment_group: regroup(s(&e, "installment_group")),
                    installment_num: i(&e, "installment_num"),
                    installment_total: i(&e, "installment_total"),
                    transfer_group: regroup(s(&e, "transfer_group")),
                    card_id: i(&e, "card_id").and_then(|old| card_map.get(&old).copied()),
                    created_at: i(&e, "created_at").unwrap_or(now),
                };
                if entry.card_id.is_some() && self.fin_invoice_entry(entry.card_id.unwrap_or(0), &entry.month)?.is_some() {
                    continue;
                }
                self.fin_insert_row(&entry)?;
                count += 1;
            }
            for k in rows("fin_recurrence_skip") {
                if let (Some(g), Some(m)) = (s(&k, "recurrence_group"), s(&k, "month")) {
                    self.conn.execute(
                        "INSERT OR IGNORE INTO fin_recurrence_skip (recurrence_group, month) VALUES (?1, ?2)",
                        params![format!("{prefix}{g}"), m],
                    )?;
                }
            }
            for b in rows("fin_budget") {
                if let Some(cat) = i(&b, "category_id").and_then(|old| cat_map.get(&old).copied()) {
                    self.fin_set_budget(cat, f(&b, "monthly_limit"))?;
                }
            }
            for p in rows("fin_card_purchase") {
                let Some(card) = i(&p, "card_id").and_then(|old| card_map.get(&old).copied()) else { continue };
                self.conn.execute(
                    "INSERT INTO fin_card_purchase (card_id, description, amount, date, invoice_month, category_id, \
                     installment_group, installment_num, installment_total, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        card,
                        s(&p, "description").unwrap_or_default(),
                        f(&p, "amount"),
                        s(&p, "date").unwrap_or_else(today_key),
                        s(&p, "invoice_month").unwrap_or_else(current_month),
                        i(&p, "category_id").and_then(|old| cat_map.get(&old).copied()),
                        regroup(s(&p, "installment_group")),
                        i(&p, "installment_num"),
                        i(&p, "installment_total"),
                        i(&p, "created_at").unwrap_or(now),
                    ],
                )?;
            }
            let mut goal_map = HashMap::new();
            for g in rows("fin_goal") {
                let goal = Goal {
                    id: 0,
                    name: s(&g, "name").unwrap_or_else(|| "Meta".into()),
                    target: f(&g, "target"),
                    saved: 0.0,
                    deadline: s(&g, "deadline"),
                    start: s(&g, "start").unwrap_or_else(current_month),
                    state: GoalState::from_code(i(&g, "state").unwrap_or(0)),
                    done_at: i(&g, "done_at"),
                    icon: s(&g, "icon").unwrap_or_else(|| "🎯".into()),
                    color: i(&g, "color").unwrap_or(1008),
                };
                goal_map.insert(i(&g, "id").unwrap_or(0), self.fin_insert_goal(&goal)?);
            }
            for d in rows("fin_goal_deposit") {
                let Some(goal) = i(&d, "goal_id").and_then(|old| goal_map.get(&old).copied()) else { continue };
                self.conn.execute(
                    "INSERT INTO fin_goal_deposit (goal_id, amount, date, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        goal,
                        f(&d, "amount"),
                        s(&d, "date").unwrap_or_else(today_key),
                        s(&d, "note").unwrap_or_default(),
                        i(&d, "created_at").unwrap_or(now)
                    ],
                )?;
            }
            Ok(count)
        })
    }
}

const FIN_TABLES: [&str; 9] = [
    "fin_account",
    "fin_category",
    "fin_card",
    "fin_entry",
    "fin_recurrence_skip",
    "fin_budget",
    "fin_card_purchase",
    "fin_goal",
    "fin_goal_deposit",
];

/// Chave de ordenação sem acentos nem maiúsculas ("Água" fica entre os "A").
pub fn sort_key(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// "Notebook (3/10)" → "Notebook".
pub fn strip_installment_suffix(description: &str) -> String {
    let d = description.trim();
    if let Some(start) = d.rfind(" (") {
        let tail = &d[start + 2..];
        if let Some(inner) = tail.strip_suffix(')') {
            if let Some((a, b)) = inner.split_once('/') {
                if a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok() {
                    return d[..start].to_string();
                }
            }
        }
    }
    d.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        parse_date(s).unwrap()
    }

    #[test]
    fn meses() {
        assert_eq!(add_months("2026-11", 2), "2027-01");
        assert_eq!(add_months("2026-01", -1), "2025-12");
        assert_eq!(days_in_month("2028-02"), 29);
        assert_eq!(day_in_month("2026-02", 31), "2026-02-28");
        assert_eq!(months_between("2026-11", "2027-02"), Some(3));
        assert_eq!(month_label("2026-03"), "Março de 2026");
        assert_eq!(month_short("2026-03"), "mar/26");
        assert_eq!(strip_installment_suffix("TV (3/10)"), "TV");
        assert_eq!(strip_installment_suffix("Plano (anual)"), "Plano (anual)");
    }

    #[test]
    fn mes_da_fatura() {
        // Fecha dia 3, vence dia 10: compra até o dia 3 cai na fatura do mesmo mês.
        assert_eq!(invoice_month(d("2026-11-02"), 3, 10), "2026-11");
        assert_eq!(invoice_month(d("2026-11-04"), 3, 10), "2026-12");
        // Fecha dia 25, vence dia 5 do mês seguinte.
        assert_eq!(invoice_month(d("2026-11-20"), 25, 5), "2026-12");
        assert_eq!(invoice_month(d("2026-11-26"), 25, 5), "2027-01");
    }

    #[test]
    fn parcelas_somam_o_total() {
        let p = split_installments(100.0, 3);
        assert_eq!(p, vec![33.33, 33.33, 33.34]);
        assert!((p.iter().sum::<f64>() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn categorias_padrao() {
        let db = Db::open_in_memory().unwrap();
        let cats = db.fin_categories().unwrap();
        assert!(cats.iter().any(|c| c.name == "Salário" && c.kind == Kind::Income));
        assert!(cats.iter().any(|c| c.name == "Mercado" && c.kind == Kind::Expense));
        let expense: Vec<&str> = cats.iter().filter(|c| c.kind == Kind::Expense).map(|c| c.name.as_str()).collect();
        assert_eq!(&expense[..2], &["Água", "Alimentação"]);
    }

    #[test]
    fn recorrencia_se_propaga_e_respeita_pulos() {
        let db = Db::open_in_memory().unwrap();
        let mut e = Entry::new(Kind::Expense, "Aluguel", 1500.0, "2026-01");
        e.due_date = Some("2026-01-31".into());
        let id = db.fin_add_entry(&e, Repeat::Monthly(None)).unwrap();
        let feb = db.fin_entries("2026-02").unwrap();
        assert_eq!(feb.len(), 1);
        assert_eq!(feb[0].due_date.as_deref(), Some("2026-02-28"));
        assert_eq!(feb[0].recurrence_group, Some(format!("r{id}")));

        // Mudança de valor em fevereiro segue para março.
        let mut f = feb[0].clone();
        f.amount = 1600.0;
        db.fin_update_entry(&f, Scope::This).unwrap();
        assert_eq!(db.fin_entries("2026-03").unwrap()[0].amount, 1600.0);

        // Apagar só abril não faz abril voltar.
        let apr = db.fin_entries("2026-04").unwrap();
        db.fin_delete_entry(apr[0].id, Scope::This).unwrap();
        assert!(db.fin_entries("2026-04").unwrap().is_empty());
        assert_eq!(db.fin_entries("2026-05").unwrap().len(), 1);

        // Parar a série em maio.
        db.fin_end_recurrence(&format!("r{id}"), "2026-05").unwrap();
        assert!(db.fin_entries("2026-06").unwrap().is_empty());
    }

    #[test]
    fn recorrencia_com_numero_de_vezes() {
        let db = Db::open_in_memory().unwrap();
        let e = Entry::new(Kind::Expense, "Dívida", 100.0, "2026-01");
        db.fin_add_entry(&e, Repeat::Monthly(Some(3))).unwrap();
        assert_eq!(db.fin_entries("2026-03").unwrap().len(), 1);
        assert!(db.fin_entries("2026-04").unwrap().is_empty());
    }

    #[test]
    fn alterar_proximas_ocorrencias() {
        let db = Db::open_in_memory().unwrap();
        let e = Entry::new(Kind::Expense, "Academia", 90.0, "2026-01");
        db.fin_add_entry(&e, Repeat::Monthly(None)).unwrap();
        db.fin_entries("2026-03").unwrap();
        let mut feb = db.fin_entries("2026-02").unwrap()[0].clone();
        feb.amount = 110.0;
        db.fin_update_entry(&feb, Scope::Following).unwrap();
        assert_eq!(db.fin_entries("2026-01").unwrap()[0].amount, 90.0);
        assert_eq!(db.fin_entries("2026-03").unwrap()[0].amount, 110.0);
    }

    #[test]
    fn parcelamento() {
        let db = Db::open_in_memory().unwrap();
        let mut e = Entry::new(Kind::Expense, "Geladeira", 250.0, "2026-11");
        e.due_date = Some("2026-11-15".into());
        db.fin_add_entry(&e, Repeat::Installments(4)).unwrap();
        let feb = db.fin_entries("2027-02").unwrap();
        assert_eq!(feb.len(), 1);
        assert_eq!(feb[0].description, "Geladeira (4/4)");
        assert_eq!(feb[0].due_date.as_deref(), Some("2027-02-15"));
        assert!(db.fin_entries("2027-03").unwrap().is_empty());
        db.fin_delete_entry(feb[0].id, Scope::All).unwrap();
        assert!(db.fin_entries("2026-11").unwrap().is_empty());
    }

    #[test]
    fn transferencia_nao_conta_nos_totais_e_mexe_nos_saldos() {
        let db = Db::open_in_memory().unwrap();
        let a = db.fin_insert_account("Banco", 1000.0, 1003).unwrap();
        let b = db.fin_insert_account("Carteira", 0.0, 1002).unwrap();
        let id = db.fin_transfer(a, b, 200.0, "2026-11-10", "").unwrap();
        let t = db.fin_month_totals("2026-11").unwrap();
        assert_eq!(t, MonthTotals::default());
        let bal = db.fin_account_balances("2026-11").unwrap();
        assert_eq!(bal.iter().find(|x| x.account.id == a).unwrap().current, 800.0);
        assert_eq!(bal.iter().find(|x| x.account.id == b).unwrap().current, 200.0);
        db.fin_delete_entry(id, Scope::This).unwrap();
        assert!(db.fin_entries("2026-11").unwrap().is_empty());
    }

    #[test]
    fn totais_saldo_e_fora_dos_totais() {
        let db = Db::open_in_memory().unwrap();
        let acc = db.fin_insert_account("Banco", 100.0, 1003).unwrap();
        let mut salary = Entry::new(Kind::Income, "Salário", 3000.0, "2026-11");
        salary.account_id = Some(acc);
        salary.paid = true;
        db.fin_add_entry(&salary, Repeat::Once).unwrap();
        let mut rent = Entry::new(Kind::Expense, "Aluguel", 1000.0, "2026-11");
        rent.account_id = Some(acc);
        db.fin_add_entry(&rent, Repeat::Once).unwrap();
        let mut boleto = Entry::new(Kind::Income, "Boleto da tia", 50.0, "2026-11");
        boleto.account_id = Some(acc);
        boleto.paid = true;
        boleto.excluded = true;
        db.fin_add_entry(&boleto, Repeat::Once).unwrap();

        let t = db.fin_month_totals("2026-11").unwrap();
        assert_eq!(t.income_total, 3000.0);
        assert_eq!(t.expense_total, 1000.0);
        assert_eq!(t.balance(), 3000.0);
        assert_eq!(t.available(), 2000.0);
        let bal = &db.fin_account_balances("2026-11").unwrap()[0];
        assert_eq!(bal.current, 3150.0);
        assert_eq!(bal.forecast, 2150.0);
        let o = db.fin_overview("2026-11").unwrap();
        assert_eq!(o.forecast_end, bal.forecast, "painel e contas usam a mesma previsão");
    }

    #[test]
    fn atrasadas_de_meses_anteriores() {
        let db = Db::open_in_memory().unwrap();
        let e = Entry::new(Kind::Expense, "Luz", 120.0, "2026-09");
        db.fin_add_entry(&e, Repeat::Monthly(None)).unwrap();
        db.fin_entries("2026-11").unwrap();
        let overdue = db.fin_overdue_before("2026-11").unwrap();
        assert_eq!(overdue.len(), 1, "só a ocorrência em aberto mais antiga da série");
        assert_eq!(overdue[0].month, "2026-09");
    }

    #[test]
    fn cartao_fatura_e_orcamento() {
        let db = Db::open_in_memory().unwrap();
        let acc = db.fin_insert_account("Banco", 0.0, 1003).unwrap();
        let mercado = db.fin_categories().unwrap().into_iter().find(|c| c.name == "Mercado").unwrap().id;
        let mut card = Card::new("Roxinho");
        card.closing_day = 3;
        card.due_day = 10;
        card.account_id = Some(acc);
        card.credit_limit = 1000.0;
        let cid = db.fin_insert_card(&card).unwrap();
        db.fin_add_purchase(cid, "Feira", 90.0, "2026-11-02", Some(mercado), 1).unwrap();
        db.fin_add_purchase(cid, "Notebook", 300.0, "2026-11-02", None, 3).unwrap();

        let inv = db.fin_invoice_entry(cid, "2026-11").unwrap().unwrap();
        assert_eq!(inv.amount, 190.0);
        assert_eq!(inv.due_date.as_deref(), Some("2026-11-10"));
        assert_eq!(db.fin_invoice_entry(cid, "2027-01").unwrap().unwrap().amount, 100.0);
        assert_eq!(db.fin_card_summaries("2026-11").unwrap()[0].open_total, 390.0);

        // Fatura conta como despesa; nas categorias entram as compras.
        assert_eq!(db.fin_month_totals("2026-11").unwrap().expense_total, 190.0);
        let by_cat = db.fin_spending_by_category("2026-11").unwrap();
        assert_eq!(by_cat.get(&Some(mercado)), Some(&90.0));
        assert_eq!(by_cat.get(&None), Some(&100.0));
        db.fin_set_budget(mercado, 300.0).unwrap();
        let budget = db.fin_budget("2026-11").unwrap();
        assert_eq!(budget[0].spent, 90.0);

        // Pagar a fatura tira do limite comprometido e do saldo da conta.
        db.fin_set_paid(inv.id, true, Some("2026-11-10")).unwrap();
        let s = &db.fin_card_summaries("2026-11").unwrap()[0];
        assert!(s.invoice_paid);
        assert_eq!(s.open_total, 200.0);
        assert_eq!(db.fin_account_balances("2026-11").unwrap()[0].current, -190.0);

        // Apagar o notebook inteiro limpa as faturas futuras.
        let notebook = db.fin_purchases(cid, "2026-12").unwrap()[0].id;
        db.fin_delete_purchase(notebook, true).unwrap();
        assert!(db.fin_invoice_entry(cid, "2027-01").unwrap().is_none());
        assert_eq!(db.fin_invoice_entry(cid, "2026-11").unwrap().unwrap().amount, 90.0);
    }

    #[test]
    fn metas_com_depositos_e_ritmo() {
        let db = Db::open_in_memory().unwrap();
        let mut g = Goal::new("Viagem", 1200.0);
        g.start = "2026-01".into();
        g.deadline = Some("2026-12".into());
        let id = db.fin_insert_goal(&g).unwrap();
        db.fin_add_deposit(id, 300.0, "2026-02-01", "").unwrap();
        let goal = db.fin_goal(id).unwrap().unwrap();
        assert_eq!(goal.saved, 300.0);
        let st = goal_status(&goal, "2026-07");
        assert_eq!(st.pace, Pace::Behind); // esperado: 1200 * 6/11
        assert_eq!(st.months_left, Some(5));
        assert!((st.per_month - 180.0).abs() < 1e-9);

        let dep = db.fin_add_deposit(id, 900.0, "2026-03-01", "bônus").unwrap();
        assert_eq!(dep, 1200.0);
        assert_eq!(db.fin_goal(id).unwrap().unwrap().state, GoalState::Done);
        let last = db.fin_deposits(id).unwrap()[0].id;
        db.fin_delete_deposit(last).unwrap();
        assert_eq!(db.fin_goal(id).unwrap().unwrap().state, GoalState::Active);
    }

    #[test]
    fn exporta_e_importa() {
        let db = Db::open_in_memory().unwrap();
        let acc = db.fin_insert_account("Banco", 10.0, 1003).unwrap();
        let mut e = Entry::new(Kind::Expense, "Internet", 99.9, "2026-11");
        e.account_id = Some(acc);
        db.fin_add_entry(&e, Repeat::Monthly(None)).unwrap();
        let cid = db.fin_insert_card(&Card::new("Visa")).unwrap();
        db.fin_add_purchase(cid, "Livro", 50.0, "2026-11-01", None, 1).unwrap();
        let gid = db.fin_insert_goal(&Goal::new("Reserva", 5000.0)).unwrap();
        db.fin_add_deposit(gid, 100.0, "2026-11-01", "").unwrap();
        let data = db.fin_export().unwrap();

        let other = Db::open_in_memory().unwrap();
        other.fin_delete_all().unwrap();
        assert_eq!(other.fin_import(&data).unwrap(), 2);
        assert_eq!(other.fin_accounts().unwrap()[0].name, "Banco");
        assert_eq!(other.fin_entries("2026-12").unwrap().len(), 1, "série continua depois de importar");
        assert_eq!(other.fin_goals().unwrap()[0].saved, 100.0);
        let card = other.fin_cards().unwrap()[0].id;
        assert_eq!(other.fin_card_summaries("2026-11").unwrap()[0].open_total, 50.0);
        assert_eq!(other.fin_purchases(card, "2026-11").unwrap().len(), 1);
    }
}
