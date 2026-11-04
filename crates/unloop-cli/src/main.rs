//! unloop-cli: manipula os dados do UnLoop pela linha de comando.
//!
//! Feita para uso humano e para agentes (Claude Code): todo comando aceita `--json`.
//! Usa o mesmo banco do app (ver `unloop-cli where`). Apagar exige `--yes`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use unloop_core::analysis;
use unloop_core::backup;
use unloop_core::calc::{entry_resets, HabitBundle};
use unloop_core::checkin::{self, day_key, describe_mask, local_date, NaiveDate};
use unloop_core::db::Db;
use unloop_core::format::{format_compact, format_money, format_number};
use unloop_core::model::*;
use unloop_core::pomodoro::{self, Period};
use unloop_core::tasks::{self, TaskView};

const APP_ID: &str = "io.github.joaovmourasouza.UnLoop";

type Res<T> = Result<T, String>;

fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> Res<T> {
    r.map_err(|e| e.to_string())
}

// ===================================================================== CLI

#[derive(Parser)]
#[command(
    name = "unloop-cli",
    version,
    about = "Hábitos, diário, notas, tarefas e foco do UnLoop pela linha de comando.",
    after_help = "Dica: use --json para saída estruturada. Prazos aceitam: hoje, amanhã, +3, seg..dom, 2026-03-12, 12/03, com hora opcional (14:30)."
)]
struct Cli {
    /// Caminho do banco (padrão: o mesmo do app; também lê $UNLOOP_DB)
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Saída em JSON
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Retrato completo para análise: hábitos com estatísticas, notas, diário recente, tarefas e foco
    Context {
        /// Dias de diário/check-ins incluídos
        #[arg(long, default_value_t = 30)]
        days: i64,
    },
    /// Hábitos
    #[command(subcommand)]
    Habit(HabitCmd),
    /// Diário (recaídas e anotações) de hábitos de parar
    #[command(subcommand)]
    Diary(DiaryCmd),
    /// Marca/registra progresso num hábito de check diário
    Checkin(CheckinArgs),
    /// Notas dos hábitos: gatilhos, o que ajuda, notas e análises
    #[command(subcommand)]
    Note(NoteCmd),
    /// Regras do hábito (permitido / não permitido)
    #[command(subcommand)]
    Rule(RuleCmd),
    /// Tarefas
    #[command(subcommand)]
    Task(TaskCmd),
    /// Listas de tarefas
    #[command(subcommand)]
    List(ListCmd),
    /// Sessões de foco (Pomodoro)
    #[command(subcommand)]
    Focus(FocusCmd),
    /// Exporta um backup JSON completo para o stdout (redirecione: `> backup.json`) ou --out
    Export {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Mostra qual banco está sendo usado
    Where,
}

#[derive(Subcommand)]
enum HabitCmd {
    /// Lista os hábitos
    List,
    /// Detalhes e estatísticas de um hábito (id ou nome)
    Show { habit: String },
}

#[derive(Subcommand)]
enum DiaryCmd {
    /// Entradas do diário de um hábito (mais recentes primeiro)
    List {
        habit: String,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Adiciona uma entrada (anotação; use --reset para recaída)
    Add {
        habit: String,
        /// Foi uma recaída (reinicia a sequência)
        #[arg(long)]
        reset: bool,
        /// Quando: "hoje 14:30", "2026-03-12 09:00"… (padrão: agora)
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        money: Option<f64>,
        #[arg(long)]
        minutes: Option<f64>,
        /// Regra relacionada (id); ela decide se reinicia
        #[arg(long)]
        rule: Option<i64>,
    },
    /// Apaga uma entrada
    Delete {
        id: i64,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
struct CheckinArgs {
    habit: String,
    /// Dia (padrão: hoje). Ex.: ontem, 2026-03-12
    #[arg(long)]
    date: Option<String>,
    /// Soma ao progresso do dia (hábitos com meta)
    #[arg(long, conflicts_with_all = ["set", "undo"])]
    add: Option<f64>,
    /// Define o total do dia (hábitos com meta)
    #[arg(long, conflicts_with = "undo")]
    set: Option<f64>,
    /// Desfaz o registro do dia
    #[arg(long)]
    undo: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum NoteKindArg {
    Note,
    Trigger,
    Helper,
    Insight,
}

impl From<NoteKindArg> for NoteKind {
    fn from(k: NoteKindArg) -> Self {
        match k {
            NoteKindArg::Note => NoteKind::Note,
            NoteKindArg::Trigger => NoteKind::Trigger,
            NoteKindArg::Helper => NoteKind::Helper,
            NoteKindArg::Insight => NoteKind::Insight,
        }
    }
}

#[derive(Subcommand)]
enum NoteCmd {
    /// Lista notas (filtros opcionais)
    List {
        #[arg(long)]
        habit: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<NoteKindArg>,
    },
    /// Mostra uma nota
    Show { id: i64 },
    /// Cria uma nota. Tipos: note, trigger (me leva a fazer), helper (me ajuda), insight (análise)
    Add {
        habit: String,
        #[arg(long, value_enum)]
        kind: NoteKindArg,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        /// Lê o corpo (Markdown) de um arquivo; "-" lê da entrada padrão
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Tags separadas por vírgula
        #[arg(long)]
        tags: Option<String>,
        /// Quem escreveu: user ou claude
        #[arg(long, default_value = "user")]
        source: String,
    },
    /// Edita uma nota
    Edit {
        id: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        tags: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<NoteKindArg>,
    },
    /// Apaga uma nota
    Delete {
        id: i64,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum RuleKindArg {
    Allowed,
    Forbidden,
}

#[derive(Subcommand)]
enum RuleCmd {
    /// Regras de um hábito
    List { habit: String },
    /// Adiciona regra: allowed (não reinicia) ou forbidden (reinicia)
    Add {
        habit: String,
        #[arg(long, value_enum)]
        kind: RuleKindArg,
        text: String,
    },
    /// Apaga uma regra
    Delete {
        id: i64,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ViewArg {
    Today,
    Week,
    Overdue,
    Inbox,
    All,
    Done,
}

impl From<ViewArg> for TaskView {
    fn from(v: ViewArg) -> Self {
        match v {
            ViewArg::Today => TaskView::Today,
            ViewArg::Week => TaskView::Next7,
            ViewArg::Overdue => TaskView::Overdue,
            ViewArg::Inbox => TaskView::Inbox,
            ViewArg::All => TaskView::All,
            ViewArg::Done => TaskView::Done,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum PriorityArg {
    None,
    Low,
    Medium,
    High,
}

impl From<PriorityArg> for Priority {
    fn from(p: PriorityArg) -> Self {
        match p {
            PriorityArg::None => Priority::Unset,
            PriorityArg::Low => Priority::Low,
            PriorityArg::Medium => Priority::Medium,
            PriorityArg::High => Priority::High,
        }
    }
}

#[derive(Args)]
struct TaskFields {
    /// Descrição (Markdown)
    #[arg(long)]
    desc: Option<String>,
    /// Lê a descrição de um arquivo; "-" lê da entrada padrão
    #[arg(long, conflicts_with = "desc")]
    desc_file: Option<PathBuf>,
    /// Prazo: hoje, amanhã 14:00, +3, sex, 2026-03-12… ("nenhum" remove)
    #[arg(long)]
    due: Option<String>,
    #[arg(long, value_enum)]
    priority: Option<PriorityArg>,
    /// Tags separadas por vírgula (substitui as atuais)
    #[arg(long)]
    tags: Option<String>,
    /// Lista (nome ou id; criada se não existir). "inbox" = Caixa de entrada
    #[arg(long)]
    list: Option<String>,
    /// Hábito relacionado (id ou nome; "nenhum" remove)
    #[arg(long)]
    habit: Option<String>,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Lista tarefas de uma visão
    List {
        #[arg(long, value_enum, default_value = "all")]
        view: ViewArg,
        /// Só de uma lista (nome ou id)
        #[arg(long)]
        list: Option<String>,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Mostra uma tarefa completa
    Show { id: i64 },
    /// Cria uma tarefa
    Add {
        title: String,
        #[command(flatten)]
        fields: TaskFields,
        /// Subtarefa (pode repetir)
        #[arg(long = "subtask")]
        subtasks: Vec<String>,
    },
    /// Edita uma tarefa
    Edit {
        id: i64,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        fields: TaskFields,
    },
    /// Conclui uma tarefa
    Done { id: i64 },
    /// Reabre uma tarefa
    Undone { id: i64 },
    /// Apaga uma tarefa
    Delete {
        id: i64,
        #[arg(long)]
        yes: bool,
    },
    /// Subtarefas
    #[command(subcommand)]
    Sub(SubCmd),
}

#[derive(Subcommand)]
enum SubCmd {
    /// Adiciona subtarefa
    Add { task: i64, title: String },
    /// Marca subtarefa como feita
    Done { id: i64 },
    /// Desmarca subtarefa
    Undone { id: i64 },
    /// Renomeia subtarefa
    Rename { id: i64, title: String },
    /// Apaga subtarefa
    Delete { id: i64 },
}

#[derive(Subcommand)]
enum ListCmd {
    /// Mostra as listas
    All,
    /// Cria uma lista
    Add { name: String },
    /// Renomeia uma lista
    Rename { list: String, name: String },
    /// Apaga uma lista (as tarefas vão para a Caixa de entrada)
    Delete {
        list: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum PeriodArg {
    Week,
    Month,
    Year,
}

impl From<PeriodArg> for Period {
    fn from(p: PeriodArg) -> Self {
        match p {
            PeriodArg::Week => Period::Week,
            PeriodArg::Month => Period::Month,
            PeriodArg::Year => Period::Year,
        }
    }
}

#[derive(Subcommand)]
enum FocusCmd {
    /// Sessões recentes
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Estatísticas de foco de um período
    Stats {
        #[arg(long, value_enum, default_value = "week")]
        period: PeriodArg,
    },
}

// ================================================================== Saída

struct Out {
    json: Value,
    text: String,
}

fn out(json: Value, text: impl Into<String>) -> Res<Out> {
    Ok(Out { json, text: text.into() })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let as_json = cli.json;
    match run(cli) {
        Ok(o) => {
            if as_json {
                println!("{}", serde_json::to_string_pretty(&o.json).unwrap_or_default());
            } else {
                println!("{}", o.text.trim_end());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if as_json {
                println!("{}", json!({ "error": e }));
            } else {
                eprintln!("erro: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

// =================================================================== Banco

fn candidate_paths() -> (PathBuf, PathBuf) {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let xdg = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    (
        xdg.join("unloop").join("unloop.db"),
        home.join(".var/app").join(APP_ID).join("data/unloop/unloop.db"),
    )
}

/// --db > $UNLOOP_DB > banco nativo existente > banco do Flatpak existente > nativo.
fn resolve_db(arg: Option<PathBuf>) -> PathBuf {
    if let Some(p) = arg {
        return p;
    }
    if let Some(p) = std::env::var_os("UNLOOP_DB") {
        return PathBuf::from(p);
    }
    let (native, flatpak) = candidate_paths();
    if native.exists() || !flatpak.exists() {
        native
    } else {
        flatpak
    }
}

fn open_db(path: &Path) -> Res<Db> {
    if let Some(parent) = path.parent() {
        ok(std::fs::create_dir_all(parent))?;
    }
    ok(Db::open(path))
}

// ================================================================ Busca

fn find_habit(db: &Db, query: &str) -> Res<Habit> {
    let habits = ok(db.habits())?;
    if let Ok(id) = query.trim().parse::<i64>() {
        return habits.into_iter().find(|h| h.id == id).ok_or_else(|| format!("hábito {id} não existe"));
    }
    let q = query.trim().to_lowercase();
    if let Some(h) = habits.iter().find(|h| h.name.to_lowercase() == q) {
        return Ok(h.clone());
    }
    let matches: Vec<&Habit> = habits.iter().filter(|h| h.name.to_lowercase().contains(&q)).collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("nenhum hábito com \"{query}\" (veja `unloop-cli habit list`)")),
        many => Err(format!(
            "\"{query}\" é ambíguo: {}",
            many.iter().map(|h| format!("#{} {}", h.id, h.name)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn bundle(db: &Db, habit: &Habit) -> Res<HabitBundle> {
    ok(db.bundle(habit.id))?.ok_or_else(|| format!("hábito {} não existe", habit.id))
}

fn find_list(db: &Db, query: &str, create: bool) -> Res<Option<i64>> {
    let q = query.trim();
    if q.eq_ignore_ascii_case("inbox") || q.eq_ignore_ascii_case("caixa de entrada") || q.is_empty() {
        return Ok(None);
    }
    let lists = ok(db.task_lists())?;
    if let Ok(id) = q.parse::<i64>() {
        return lists.iter().find(|l| l.id == id).map(|l| Some(l.id)).ok_or_else(|| format!("lista {id} não existe"));
    }
    if let Some(l) = lists.iter().find(|l| l.name.eq_ignore_ascii_case(q)) {
        return Ok(Some(l.id));
    }
    if create {
        Ok(Some(ok(db.insert_task_list(q, 1003))?))
    } else {
        Err(format!("lista \"{q}\" não existe"))
    }
}

/// Texto de `--body`/`--desc` ou de arquivo. `-` lê da entrada padrão (funciona também
/// dentro do Flatpak, onde a CLI não enxerga /tmp nem a home).
fn read_text(inline: Option<String>, file: Option<PathBuf>) -> Res<Option<String>> {
    match (inline, file) {
        (Some(t), _) => Ok(Some(t)),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut text = String::new();
            ok(std::io::Read::read_to_string(&mut std::io::stdin(), &mut text))?;
            Ok(Some(text))
        }
        (None, Some(path)) => ok(std::fs::read_to_string(&path)).map(Some),
        (None, None) => Ok(None),
    }
}

fn require_yes(yes: bool, what: &str) -> Res<()> {
    if yes {
        Ok(())
    } else {
        Err(format!("isso apaga {what}; repita com --yes para confirmar"))
    }
}

// =============================================================== JSON

fn iso(ms: i64) -> String {
    checkin::local_datetime_string(ms)
}

fn habit_json(b: &HabitBundle, now: i64, today: NaiveDate, currency: &str) -> Value {
    let h = &b.habit;
    let mut v = json!({
        "id": h.id,
        "name": h.name,
        "kind": h.kind.legacy(),
        "icon": icon(h.icon).emoji,
        "start": iso(h.quit_at),
    });
    if h.is_build() {
        let s = b.checkin_stats(today);
        v["schedule"] = json!(describe_mask(h.schedule_days));
        v["dailyTarget"] = json!(if h.has_target() { Some(format!("{} {}", format_number(h.daily_target), h.target_unit)) } else { None });
        v["stats"] = json!({
            "currentStreak": s.current_streak,
            "bestStreak": s.best_streak,
            "completionRate": (s.completion_rate() * 1000.0).round() / 10.0,
            "scheduledToday": s.scheduled_today,
            "doneToday": s.done_today,
            "amountToday": b.amount_on(today),
            "trophies": s.trophies_achieved(),
        });
    } else {
        let snap = b.snapshot(now);
        let bal = &snap.balances;
        let by_period: Vec<Value> = analysis::relapses_by_period_of_day(&b.entries)
            .iter()
            .map(|(p, c)| json!({ "period": p, "count": c }))
            .collect();
        let by_weekday: Vec<Value> = checkin::WEEKDAY_SHORT
            .iter()
            .zip(analysis::relapses_by_weekday(&b.entries))
            .map(|(d, c)| json!({ "day": d, "count": c }))
            .collect();
        let rules: Vec<Value> = b
            .rules
            .iter()
            .map(|r| json!({ "id": r.id, "kind": r.kind.legacy(), "text": r.text, "uses": b.rule_usage(r.id) }))
            .collect();
        let longest = snap.periods.longest().map(format_compact);
        let average = snap.periods.average().map(format_compact);
        let goal = snap.goal.as_ref().map(|g| g.name.clone());
        v["stats"] = json!({
            "streakStart": iso(snap.start),
            "currentStreak": format_compact(snap.elapsed()),
            "currentStreakDays": snap.elapsed() as f64 / 86_400_000.0,
            "relapses": snap.periods.count(),
            "longestPeriod": longest,
            "averagePeriod": average,
            "currentGoal": goal,
            "trophies": snap.trophies_achieved(),
            "moneySavedNet": format_money(bal.net_saved(), currency),
            "timeRecovered": format_compact(bal.net_time_ms()),
            "relapsesByWeekday": by_weekday,
            "relapsesByPeriodOfDay": by_period,
            "riskiestWeekday": analysis::riskiest_weekday(&b.entries),
        });
        v["rules"] = json!(rules);
    }
    v["reasons"] = json!(b.reasons.iter().map(|r| r.text.clone()).collect::<Vec<_>>());
    v
}

fn note_json(n: &HabitNote) -> Value {
    json!({
        "id": n.id, "habitId": n.habit_id, "kind": n.kind.legacy(), "title": n.title, "body": n.body,
        "tags": n.tags, "source": n.source, "createdAt": iso(n.created_at), "updatedAt": iso(n.updated_at),
    })
}

fn entry_json(e: &DiaryEntry, b: &HabitBundle) -> Value {
    let rule = b.rule(e.rule_id).map(|r| json!({ "id": r.id, "kind": r.kind.legacy(), "text": r.text }));
    json!({
        "id": e.id, "at": iso(e.occurred_at), "reset": e.is_reset, "note": e.note,
        "moneySpent": e.money_spent, "minutesSpent": e.time_spent_ms / 60_000,
        "rule": rule,
    })
}

fn task_json(t: &Task, lists: &[TaskList], today: NaiveDate, now: i64) -> Value {
    let (done, total) = t.subtask_progress();
    let list = t.list_id.and_then(|id| lists.iter().find(|l| l.id == id)).map(|l| l.name.clone());
    let due = t.due_at.map(|ms| if t.due_has_time { iso(ms) } else { day_key(local_date(ms)) });
    let subtasks: Vec<Value> =
        t.subtasks.iter().map(|s| json!({ "id": s.id, "title": s.title, "done": s.done })).collect();
    json!({
        "id": t.id,
        "title": t.title,
        "description": t.description,
        "list": list,
        "due": due,
        "dueLabel": tasks::due_label(t, today),
        "overdue": tasks::is_overdue(t, today, now),
        "priority": t.priority.legacy(),
        "tags": t.tags,
        "done": t.is_done(),
        "doneAt": t.done_at.map(iso),
        "habitId": t.habit_id,
        "subtasks": subtasks,
        "subtaskProgress": format!("{done}/{total}"),
    })
}

fn task_line(t: &Task, lists: &[TaskList], today: NaiveDate) -> String {
    let mut parts = vec![format!("{} #{} {}{}", if t.is_done() { "[x]" } else { "[ ]" }, t.id, t.priority.emoji(), t.title)];
    if let Some(label) = tasks::due_label(t, today) {
        parts.push(label);
    }
    if let Some(l) = t.list_id.and_then(|id| lists.iter().find(|l| l.id == id)) {
        parts.push(l.name.clone());
    }
    if !t.tags.is_empty() {
        parts.push(t.tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" "));
    }
    let (done, total) = t.subtask_progress();
    if total > 0 {
        parts.push(format!("{done}/{total}"));
    }
    parts.join(" · ")
}

// ============================================================== Comandos

fn run(cli: Cli) -> Res<Out> {
    let path = resolve_db(cli.db);
    if let Command::Where = cli.command {
        let (native, flatpak) = candidate_paths();
        return out(
            json!({ "using": path, "native": native, "nativeExists": native.exists(), "flatpak": flatpak, "flatpakExists": flatpak.exists() }),
            format!(
                "Usando: {}\nNativo:  {} ({})\nFlatpak: {} ({})\nUse --db ou $UNLOOP_DB para escolher outro.",
                path.display(),
                native.display(),
                if native.exists() { "existe" } else { "não existe" },
                flatpak.display(),
                if flatpak.exists() { "existe" } else { "não existe" }
            ),
        );
    }
    let db = open_db(&path)?;
    let now = unloop_core::now_ms();
    let today = checkin::today();
    let currency = ok(db.setting("currency"))?.unwrap_or_else(|| "BRL".into());

    match cli.command {
        Command::Where => unreachable!(),
        Command::Context { days } => context(&db, now, today, &currency, days),
        Command::Habit(cmd) => habit_cmd(&db, cmd, now, today, &currency),
        Command::Diary(cmd) => diary_cmd(&db, cmd, now, today),
        Command::Checkin(args) => checkin_cmd(&db, args, today),
        Command::Note(cmd) => note_cmd(&db, cmd),
        Command::Rule(cmd) => rule_cmd(&db, cmd),
        Command::Task(cmd) => task_cmd(&db, cmd, now, today),
        Command::List(cmd) => list_cmd(&db, cmd),
        Command::Focus(cmd) => focus_cmd(&db, cmd, today),
        Command::Export { out: file } => {
            let data = ok(backup::export_backup(&db, env!("CARGO_PKG_VERSION")))?;
            let text = ok(serde_json::to_string_pretty(&data))?;
            match file {
                Some(p) => {
                    ok(std::fs::write(&p, text))?;
                    out(json!({ "saved": p }), format!("Backup salvo em {}", p.display()))
                }
                None => out(data, text),
            }
        }
    }
}

fn context(db: &Db, now: i64, today: NaiveDate, currency: &str, days: i64) -> Res<Out> {
    let since = now - days.max(1) * 86_400_000;
    let mut habits = Vec::new();
    let mut text = String::new();
    for h in ok(db.habits())? {
        let b = bundle(db, &h)?;
        let mut v = habit_json(&b, now, today, currency);
        v["notes"] = json!(ok(db.notes(Some(h.id), None))?.iter().map(note_json).collect::<Vec<_>>());
        v["recentDiary"] = json!(b.entries.iter().filter(|e| e.occurred_at >= since).map(|e| entry_json(e, &b)).collect::<Vec<_>>());
        if h.is_build() {
            let since_day = local_date(since);
            v["recentCheckins"] = json!(b
                .checkins
                .iter()
                .filter(|p| checkin::parse_day(&p.day).is_some_and(|d| d >= since_day))
                .map(|p| json!({ "day": p.day, "amount": p.amount }))
                .collect::<Vec<_>>());
        }
        text.push_str(&format!("{} #{} {}\n", icon(h.icon).emoji, h.id, h.name));
        for line in analysis::summary_lines(&b, now) {
            text.push_str(&format!("   {line}\n"));
        }
        habits.push(v);
    }
    let all_tasks = ok(db.tasks())?;
    let lists = ok(db.task_lists())?;
    let view = |v: TaskView| {
        tasks::filter_view(&all_tasks, v, today, now)
            .into_iter()
            .map(|t| task_json(t, &lists, today, now))
            .collect::<Vec<_>>()
    };
    let records = ok(db.focus_sessions_since(pomodoro::week_window_start(now)))?;
    let summary = pomodoro::summarize(&records, today);
    let open = tasks::filter_view(&all_tasks, TaskView::All, today, now).len();
    text.push_str(&format!(
        "\nTarefas abertas: {open} · hoje/atrasadas: {}\nFoco hoje: {} · últimos 7 dias: {}\n",
        tasks::filter_view(&all_tasks, TaskView::Today, today, now).len(),
        format_compact(summary.today_ms),
        format_compact(summary.week_ms)
    ));
    out(
        json!({
            "generatedAt": iso(now),
            "today": day_key(today),
            "currency": currency,
            "habits": habits,
            "tasks": { "open": open, "overdue": view(TaskView::Overdue), "today": view(TaskView::Today), "next7Days": view(TaskView::Next7) },
            "focus": { "todayMinutes": summary.today_ms / 60_000, "todaySessions": summary.today_sessions, "last7DaysMinutes": summary.week_ms / 60_000 },
        }),
        text,
    )
}

fn habit_cmd(db: &Db, cmd: HabitCmd, now: i64, today: NaiveDate, currency: &str) -> Res<Out> {
    match cmd {
        HabitCmd::List => {
            let mut items = Vec::new();
            let mut text = String::new();
            for h in ok(db.habits())? {
                let b = bundle(db, &h)?;
                let status = if h.is_build() {
                    let s = b.checkin_stats(today);
                    format!("🔥 {} seguidos{}", s.current_streak, if s.done_today { " · feito hoje" } else { "" })
                } else {
                    format_compact(b.snapshot(now).elapsed())
                };
                text.push_str(&format!("#{} {} {} ({}) — {status}\n", h.id, icon(h.icon).emoji, h.name, if h.is_build() { "check" } else { "parar" }));
                items.push(json!({ "id": h.id, "name": h.name, "kind": h.kind.legacy(), "status": status }));
            }
            if items.is_empty() {
                text = "Nenhum hábito.".into();
            }
            out(json!(items), text)
        }
        HabitCmd::Show { habit } => {
            let h = find_habit(db, &habit)?;
            let b = bundle(db, &h)?;
            let mut v = habit_json(&b, now, today, currency);
            v["notes"] = json!(ok(db.notes(Some(h.id), None))?.iter().map(note_json).collect::<Vec<_>>());
            let mut text = format!("#{} {} {}\n", h.id, icon(h.icon).emoji, h.name);
            for line in analysis::summary_lines(&b, now) {
                text.push_str(&format!("  {line}\n"));
            }
            out(v, text)
        }
    }
}

fn diary_cmd(db: &Db, cmd: DiaryCmd, now: i64, today: NaiveDate) -> Res<Out> {
    match cmd {
        DiaryCmd::List { habit, limit } => {
            let h = find_habit(db, &habit)?;
            let b = bundle(db, &h)?;
            let items: Vec<&DiaryEntry> = b.entries.iter().take(limit).collect();
            let text = items
                .iter()
                .map(|e| {
                    format!(
                        "#{} {} {}{}{}",
                        e.id,
                        iso(e.occurred_at),
                        if e.is_reset { "🔁 recaída" } else { "📝" },
                        b.rule(e.rule_id).map(|r| format!(" [{} {}]", r.kind.emoji(), r.text)).unwrap_or_default(),
                        e.note.as_deref().map(|n| format!(" — {n}")).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            out(json!(items.iter().map(|e| entry_json(e, &b)).collect::<Vec<_>>()), if text.is_empty() { "Diário vazio.".into() } else { text })
        }
        DiaryCmd::Add { habit, reset, at, note, money, minutes, rule } => {
            let h = find_habit(db, &habit)?;
            if h.is_build() {
                return Err("este é um hábito de check diário; use `unloop-cli checkin`".into());
            }
            let b = bundle(db, &h)?;
            let occurred_at = match at {
                Some(text) => {
                    let (ms, has_time) =
                        tasks::parse_due(&text, today).ok_or_else(|| format!("data inválida: {text}"))?;
                    // Só a data de hoje (sem hora): usa o horário atual em vez do meio-dia.
                    if !has_time && local_date(ms) == today { now } else { ms }
                }
                None => now,
            };
            if occurred_at > now + 60_000 {
                return Err("a entrada não pode estar no futuro".into());
            }
            let rule_ref = match rule {
                Some(id) => Some(b.rules.iter().find(|r| r.id == id).ok_or_else(|| format!("regra {id} não pertence a este hábito"))?),
                None => None,
            };
            let entry = DiaryEntry {
                id: 0,
                habit_id: h.id,
                occurred_at,
                money_spent: money.unwrap_or(0.0).max(0.0),
                time_spent_ms: (minutes.unwrap_or(0.0).max(0.0) * 60_000.0).round() as i64,
                is_reset: entry_resets(rule_ref, reset),
                note,
                rule_id: rule_ref.map(|r| r.id),
            };
            let id = ok(db.insert_entry(&entry))?;
            out(
                json!({ "id": id, "reset": entry.is_reset }),
                format!("Entrada #{id} criada{}.", if entry.is_reset { " (recaída: a sequência reinicia)" } else { "" }),
            )
        }
        DiaryCmd::Delete { id, yes } => {
            require_yes(yes, &format!("a entrada #{id}"))?;
            ok(db.delete_entry(id))?;
            out(json!({ "deleted": id }), format!("Entrada #{id} apagada."))
        }
    }
}

fn checkin_cmd(db: &Db, args: CheckinArgs, today: NaiveDate) -> Res<Out> {
    let h = find_habit(db, &args.habit)?;
    if !h.is_build() {
        return Err("este é um hábito de parar; use `unloop-cli diary add`".into());
    }
    let day = match &args.date {
        Some(text) => tasks::parse_due(text, today).map(|x| local_date(x.0)).ok_or_else(|| format!("data inválida: {text}"))?,
        None => today,
    };
    if day > today {
        return Err("não dá para registrar um dia no futuro".into());
    }
    let key = day_key(day);
    let total = if args.undo {
        ok(db.set_progress(h.id, &key, 0.0))?;
        0.0
    } else if let Some(total) = args.set {
        ok(db.set_progress(h.id, &key, total))?;
        total.max(0.0)
    } else if h.has_target() {
        let delta = args.add.ok_or("hábito com meta: use --add N (soma) ou --set N (total do dia)")?;
        ok(db.add_progress(h.id, &key, delta))?
    } else {
        ok(db.set_checkin(h.id, &key, true))?;
        1.0
    };
    let done = checkin::day_done(total, h.daily_target);
    let text = if h.has_target() {
        format!("{} em {key}: {} de {} {}{}", h.name, format_number(total), format_number(h.daily_target), h.target_unit, if done { " ✓ meta batida" } else { "" })
    } else if args.undo {
        format!("{} em {key}: desmarcado", h.name)
    } else {
        format!("{} em {key}: feito ✓", h.name)
    };
    out(json!({ "habitId": h.id, "day": key, "amount": total, "done": done }), text)
}

fn note_cmd(db: &Db, cmd: NoteCmd) -> Res<Out> {
    match cmd {
        NoteCmd::List { habit, kind } => {
            let habit_id = match habit {
                Some(q) => Some(find_habit(db, &q)?.id),
                None => None,
            };
            let notes = ok(db.notes(habit_id, kind.map(NoteKind::from)))?;
            let text = notes
                .iter()
                .map(|n| format!("#{} {} [{}] {}{}", n.id, n.kind.emoji(), n.habit_id, n.title, if n.by_claude() { " 🤖" } else { "" }))
                .collect::<Vec<_>>()
                .join("\n");
            out(json!(notes.iter().map(note_json).collect::<Vec<_>>()), if text.is_empty() { "Nenhuma nota.".into() } else { text })
        }
        NoteCmd::Show { id } => {
            let n = ok(db.note(id))?.ok_or_else(|| format!("nota {id} não existe"))?;
            let text = format!("#{} {} {}\n{}\n\n{}", n.id, n.kind.emoji(), n.title, n.tags.join(", "), n.body);
            out(note_json(&n), text)
        }
        NoteCmd::Add { habit, kind, title, body, body_file, tags, source } => {
            let h = find_habit(db, &habit)?;
            let body = read_text(body, body_file)?.unwrap_or_default();
            if title.trim().is_empty() && body.trim().is_empty() {
                return Err("informe --title ou --body".into());
            }
            let mut note = HabitNote::new(h.id, kind.into(), &title, &body);
            note.tags = parse_tags(tags.as_deref().unwrap_or(""));
            note.source = source.trim().to_lowercase();
            let id = ok(db.insert_note(&note))?;
            out(json!({ "id": id }), format!("Nota #{id} criada em \"{}\".", h.name))
        }
        NoteCmd::Edit { id, title, body, body_file, tags, kind } => {
            let mut n = ok(db.note(id))?.ok_or_else(|| format!("nota {id} não existe"))?;
            if let Some(t) = title {
                n.title = t;
            }
            if let Some(b) = read_text(body, body_file)? {
                n.body = b;
            }
            if let Some(t) = tags {
                n.tags = parse_tags(&t);
            }
            if let Some(k) = kind {
                n.kind = k.into();
            }
            ok(db.update_note(&n))?;
            out(json!({ "id": id }), format!("Nota #{id} atualizada."))
        }
        NoteCmd::Delete { id, yes } => {
            require_yes(yes, &format!("a nota #{id}"))?;
            ok(db.delete_note(id))?;
            out(json!({ "deleted": id }), format!("Nota #{id} apagada."))
        }
    }
}

fn rule_cmd(db: &Db, cmd: RuleCmd) -> Res<Out> {
    match cmd {
        RuleCmd::List { habit } => {
            let h = find_habit(db, &habit)?;
            let b = bundle(db, &h)?;
            let text = b
                .rules
                .iter()
                .map(|r| format!("#{} {} {} (usada {}x)", r.id, r.kind.emoji(), r.text, b.rule_usage(r.id)))
                .collect::<Vec<_>>()
                .join("\n");
            out(
                json!(b.rules.iter().map(|r| json!({ "id": r.id, "kind": r.kind.legacy(), "text": r.text, "uses": b.rule_usage(r.id) })).collect::<Vec<_>>()),
                if text.is_empty() { "Nenhuma regra.".into() } else { text },
            )
        }
        RuleCmd::Add { habit, kind, text } => {
            let h = find_habit(db, &habit)?;
            let kind = match kind {
                RuleKindArg::Allowed => RuleKind::Allowed,
                RuleKindArg::Forbidden => RuleKind::Forbidden,
            };
            let id = ok(db.insert_rule(h.id, kind, &text))?;
            out(json!({ "id": id }), format!("Regra #{id} criada: {} {}", kind.emoji(), text.trim()))
        }
        RuleCmd::Delete { id, yes } => {
            require_yes(yes, &format!("a regra #{id}"))?;
            ok(db.delete_rule(id))?;
            out(json!({ "deleted": id }), format!("Regra #{id} apagada."))
        }
    }
}

/// Aplica os campos opcionais na tarefa.
fn apply_fields(db: &Db, t: &mut Task, f: TaskFields, today: NaiveDate) -> Res<()> {
    if let Some(d) = read_text(f.desc, f.desc_file)? {
        t.description = d;
    }
    if let Some(due) = f.due {
        if matches!(due.trim().to_lowercase().as_str(), "nenhum" | "none" | "") {
            t.due_at = None;
            t.due_has_time = false;
        } else {
            let (ms, has_time) = tasks::parse_due(&due, today).ok_or_else(|| format!("prazo inválido: {due}"))?;
            t.due_at = Some(ms);
            t.due_has_time = has_time;
        }
    }
    if let Some(p) = f.priority {
        t.priority = p.into();
    }
    if let Some(tags) = f.tags {
        t.tags = parse_tags(&tags);
    }
    if let Some(list) = f.list {
        t.list_id = find_list(db, &list, true)?;
    }
    if let Some(habit) = f.habit {
        t.habit_id = if matches!(habit.trim().to_lowercase().as_str(), "nenhum" | "none") {
            None
        } else {
            Some(find_habit(db, &habit)?.id)
        };
    }
    Ok(())
}

fn get_task(db: &Db, id: i64) -> Res<Task> {
    ok(db.task(id))?.ok_or_else(|| format!("tarefa {id} não existe"))
}

fn task_cmd(db: &Db, cmd: TaskCmd, now: i64, today: NaiveDate) -> Res<Out> {
    let lists = ok(db.task_lists())?;
    match cmd {
        TaskCmd::List { view, list, tag } => {
            let all = ok(db.tasks())?;
            let list_filter = match list {
                Some(q) => Some(find_list(db, &q, false)?),
                None => None,
            };
            let tag = tag.map(|t| t.trim().trim_start_matches('#').to_lowercase());
            let selected: Vec<&Task> = tasks::filter_view(&all, view.into(), today, now)
                .into_iter()
                .filter(|t| list_filter.map_or(true, |l| t.list_id == l))
                .filter(|t| tag.as_ref().map_or(true, |tag| t.tags.contains(tag)))
                .collect();
            let text = selected.iter().map(|t| task_line(t, &lists, today)).collect::<Vec<_>>().join("\n");
            out(
                json!(selected.iter().map(|t| task_json(t, &lists, today, now)).collect::<Vec<_>>()),
                if text.is_empty() { "Nenhuma tarefa.".into() } else { text },
            )
        }
        TaskCmd::Show { id } => {
            let t = get_task(db, id)?;
            let mut text = task_line(&t, &lists, today);
            if !t.description.is_empty() {
                text.push_str(&format!("\n\n{}", t.description));
            }
            for s in &t.subtasks {
                text.push_str(&format!("\n  {} #{} {}", if s.done { "[x]" } else { "[ ]" }, s.id, s.title));
            }
            out(task_json(&t, &lists, today, now), text)
        }
        TaskCmd::Add { title, fields, subtasks } => {
            if title.trim().is_empty() {
                return Err("o título não pode ser vazio".into());
            }
            let mut t = Task::new(&title);
            apply_fields(db, &mut t, fields, today)?;
            t.subtasks = subtasks
                .iter()
                .filter(|s| !s.trim().is_empty())
                .map(|s| Subtask { id: 0, task_id: 0, title: s.trim().to_string(), done: false, position: 0 })
                .collect();
            let id = ok(db.insert_task(&t))?;
            let saved = get_task(db, id)?;
            let lists = ok(db.task_lists())?;
            out(task_json(&saved, &lists, today, now), format!("Criada: {}", task_line(&saved, &lists, today)))
        }
        TaskCmd::Edit { id, title, fields } => {
            let mut t = get_task(db, id)?;
            if let Some(title) = title {
                if title.trim().is_empty() {
                    return Err("o título não pode ser vazio".into());
                }
                t.title = title;
            }
            apply_fields(db, &mut t, fields, today)?;
            ok(db.update_task(&t))?;
            let saved = get_task(db, id)?;
            let lists = ok(db.task_lists())?;
            out(task_json(&saved, &lists, today, now), format!("Atualizada: {}", task_line(&saved, &lists, today)))
        }
        TaskCmd::Done { id } | TaskCmd::Undone { id } if get_task(db, id).is_err() => Err(format!("tarefa {id} não existe")),
        TaskCmd::Done { id } => {
            ok(db.set_task_done(id, true))?;
            out(json!({ "id": id, "done": true }), format!("Tarefa #{id} concluída ✓"))
        }
        TaskCmd::Undone { id } => {
            ok(db.set_task_done(id, false))?;
            out(json!({ "id": id, "done": false }), format!("Tarefa #{id} reaberta."))
        }
        TaskCmd::Delete { id, yes } => {
            require_yes(yes, &format!("a tarefa #{id} e as subtarefas"))?;
            ok(db.delete_task(id))?;
            out(json!({ "deleted": id }), format!("Tarefa #{id} apagada."))
        }
        TaskCmd::Sub(sub) => match sub {
            SubCmd::Add { task, title } => {
                get_task(db, task)?;
                let id = ok(db.add_subtask(task, &title))?;
                out(json!({ "id": id, "taskId": task }), format!("Subtarefa #{id} adicionada à tarefa #{task}."))
            }
            SubCmd::Done { id } => {
                ok(db.set_subtask_done(id, true))?;
                out(json!({ "id": id, "done": true }), format!("Subtarefa #{id} feita ✓"))
            }
            SubCmd::Undone { id } => {
                ok(db.set_subtask_done(id, false))?;
                out(json!({ "id": id, "done": false }), format!("Subtarefa #{id} desmarcada."))
            }
            SubCmd::Rename { id, title } => {
                ok(db.rename_subtask(id, &title))?;
                out(json!({ "id": id }), format!("Subtarefa #{id} renomeada."))
            }
            SubCmd::Delete { id } => {
                ok(db.delete_subtask(id))?;
                out(json!({ "deleted": id }), format!("Subtarefa #{id} apagada."))
            }
        },
    }
}

fn list_cmd(db: &Db, cmd: ListCmd) -> Res<Out> {
    match cmd {
        ListCmd::All => {
            let lists = ok(db.task_lists())?;
            let tasks_all = ok(db.tasks())?;
            let count = |id: Option<i64>| tasks_all.iter().filter(|t| !t.is_done() && t.list_id == id).count();
            let mut text = format!("Caixa de entrada ({})\n", count(None));
            for l in &lists {
                text.push_str(&format!("#{} {} ({})\n", l.id, l.name, count(Some(l.id))));
            }
            out(
                json!(lists.iter().map(|l| json!({ "id": l.id, "name": l.name, "open": count(Some(l.id)) })).collect::<Vec<_>>()),
                text,
            )
        }
        ListCmd::Add { name } => {
            if name.trim().is_empty() {
                return Err("nome vazio".into());
            }
            let id = ok(db.insert_task_list(&name, 1003))?;
            out(json!({ "id": id }), format!("Lista #{id} criada."))
        }
        ListCmd::Rename { list, name } => {
            let id = find_list(db, &list, false)?.ok_or("a Caixa de entrada não pode ser renomeada")?;
            ok(db.rename_task_list(id, &name))?;
            out(json!({ "id": id }), format!("Lista #{id} renomeada."))
        }
        ListCmd::Delete { list, yes } => {
            let id = find_list(db, &list, false)?.ok_or("a Caixa de entrada não pode ser apagada")?;
            require_yes(yes, &format!("a lista #{id} (as tarefas vão para a Caixa de entrada)"))?;
            ok(db.delete_task_list(id))?;
            out(json!({ "deleted": id }), format!("Lista #{id} apagada."))
        }
    }
}

fn focus_cmd(db: &Db, cmd: FocusCmd, today: NaiveDate) -> Res<Out> {
    let records = ok(db.focus_sessions_since(0))?;
    let names: std::collections::HashMap<i64, String> = ok(db.habits())?.into_iter().map(|h| (h.id, h.name)).collect();
    let task_titles: std::collections::HashMap<i64, String> = ok(db.tasks())?.into_iter().map(|t| (t.id, t.title)).collect();
    match cmd {
        FocusCmd::List { limit } => {
            let items: Vec<&pomodoro::FocusRecord> = records.iter().take(limit).collect();
            let text = items
                .iter()
                .map(|r| {
                    format!(
                        "#{} {} {} {}{}",
                        r.id,
                        iso(r.started_at),
                        format_compact(r.duration_ms),
                        r.habit_id.and_then(|id| names.get(&id)).cloned().unwrap_or_default(),
                        r.task_id.and_then(|id| task_titles.get(&id)).map(|t| format!(" · tarefa: {t}")).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            out(
                json!(items
                    .iter()
                    .map(|r| json!({
                        "id": r.id, "start": iso(r.started_at), "minutes": r.duration_ms / 60_000,
                        "mode": if r.mode == pomodoro::Mode::Pomodoro { "pomodoro" } else { "stopwatch" },
                        "habit": r.habit_id.and_then(|id| names.get(&id)), "task": r.task_id.and_then(|id| task_titles.get(&id)),
                    }))
                    .collect::<Vec<_>>()),
                if text.is_empty() { "Nenhuma sessão.".into() } else { text },
            )
        }
        FocusCmd::Stats { period } => {
            let period: Period = period.into();
            let selected = pomodoro::in_period(&records, period, today);
            let totals = pomodoro::totals(&selected);
            let comparison = pomodoro::compare_with_previous(&records, period, today, today);
            let by_habit: Vec<Value> = pomodoro::by_habit(&selected)
                .into_iter()
                .map(|(id, ms)| json!({ "habit": id.and_then(|i| names.get(&i)).cloned().unwrap_or_else(|| "Sem hábito".into()), "minutes": ms / 60_000 }))
                .collect();
            let series: Vec<Value> = pomodoro::series(&records, period, today)
                .into_iter()
                .map(|(label, ms)| json!({ "label": label, "minutes": ms / 60_000 }))
                .collect();
            let text = format!(
                "{}: {} · {} 🍅\n{}",
                pomodoro::period_label(period, today),
                format_compact(totals.ms),
                totals.pomodoros,
                pomodoro::comparison_text(&comparison, period)
            );
            out(
                json!({
                    "period": pomodoro::period_label(period, today),
                    "minutes": totals.ms / 60_000, "pomodoros": totals.pomodoros, "sessions": totals.sessions,
                    "comparison": pomodoro::comparison_text(&comparison, period),
                    "previousMinutes": comparison.previous.ms / 60_000,
                    "byHabit": by_habit, "series": series,
                    "focusDayStreak": pomodoro::focus_day_streak(&records, today),
                }),
                text,
            )
        }
    }
}
