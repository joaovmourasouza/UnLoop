//! Tarefas: visões (Hoje, Próximos 7 dias…), ordenação e leitura de prazos.

use chrono::{Datelike, NaiveDate, TimeZone};

use crate::checkin::{local_date, WEEKDAY_SHORT};
use crate::model::Task;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskView {
    Today,
    Next7,
    Overdue,
    Inbox,
    All,
    Done,
}

impl TaskView {
    pub const ALL: [TaskView; 6] =
        [TaskView::Today, TaskView::Next7, TaskView::Overdue, TaskView::Inbox, TaskView::All, TaskView::Done];

    pub fn label(self) -> &'static str {
        match self {
            TaskView::Today => "Hoje",
            TaskView::Next7 => "Próximos 7 dias",
            TaskView::Overdue => "Atrasadas",
            TaskView::Inbox => "Caixa de entrada",
            TaskView::All => "Todas",
            TaskView::Done => "Concluídas",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            TaskView::Today => "today",
            TaskView::Next7 => "week",
            TaskView::Overdue => "overdue",
            TaskView::Inbox => "inbox",
            TaskView::All => "all",
            TaskView::Done => "done",
        }
    }

    pub fn from_key(key: &str) -> Option<TaskView> {
        TaskView::ALL.into_iter().find(|v| v.key() == key)
    }
}

/// Instante local (epoch ms) de uma data e hora.
pub fn local_ms(day: NaiveDate, hour: u32, minute: u32) -> i64 {
    let midnight = chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("meia-noite é válida");
    let time = chrono::NaiveTime::from_hms_opt(hour.min(23), minute.min(59), 0).unwrap_or(midnight);
    let naive = day.and_time(time);
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| naive.and_utc().timestamp_millis())
}

pub fn due_day(t: &Task) -> Option<NaiveDate> {
    t.due_at.map(local_date)
}

pub fn is_overdue(t: &Task, today: NaiveDate, now: i64) -> bool {
    match (t.is_done(), t.due_at) {
        (false, Some(due)) if t.due_has_time => due < now,
        (false, Some(due)) => local_date(due) < today,
        _ => false,
    }
}

/// Tarefas de uma visão, já ordenadas.
pub fn filter_view<'a>(tasks: &'a [Task], view: TaskView, today: NaiveDate, now: i64) -> Vec<&'a Task> {
    let week_end = today.checked_add_days(chrono::Days::new(6)).unwrap_or(today);
    let mut out: Vec<&Task> = tasks
        .iter()
        .filter(|t| match view {
            TaskView::Today => !t.is_done() && due_day(t).is_some_and(|d| d <= today),
            TaskView::Next7 => !t.is_done() && due_day(t).is_some_and(|d| d >= today && d <= week_end),
            TaskView::Overdue => is_overdue(t, today, now),
            TaskView::Inbox => !t.is_done() && t.list_id.is_none(),
            TaskView::All => !t.is_done(),
            TaskView::Done => t.is_done(),
        })
        .collect();
    if view == TaskView::Done {
        out.sort_by(|a, b| b.done_at.cmp(&a.done_at));
    } else {
        sort_tasks(&mut out);
    }
    out
}

/// Prazo mais próximo primeiro (sem prazo no fim), depois prioridade, posição e id.
pub fn sort_tasks(tasks: &mut [&Task]) {
    tasks.sort_by_key(|t| (t.due_at.is_none(), t.due_at.unwrap_or(0), std::cmp::Reverse(t.priority.code()), t.position, t.id));
}

/// "Hoje", "Amanhã 14:30", "Ontem", "Sex", "12/03", "12/03/2026".
pub fn due_label(t: &Task, today: NaiveDate) -> Option<String> {
    let due = t.due_at?;
    let day = local_date(due);
    let diff = (day - today).num_days();
    let mut label = match diff {
        0 => "Hoje".to_string(),
        1 => "Amanhã".to_string(),
        -1 => "Ontem".to_string(),
        2..=6 => WEEKDAY_SHORT[day.weekday().num_days_from_monday() as usize].to_string(),
        _ if day.year() == today.year() => day.format("%d/%m").to_string(),
        _ => day.format("%d/%m/%Y").to_string(),
    };
    if t.due_has_time {
        if let Some(dt) = chrono::DateTime::from_timestamp_millis(due) {
            label.push(' ');
            label.push_str(&dt.with_timezone(&chrono::Local).format("%H:%M").to_string());
        }
    }
    Some(label)
}

/// Lê um prazo digitado: "hoje", "amanhã", "+3", "seg".."dom", "2026-03-12",
/// "12/03", "12/03/2026" — opcionalmente seguido de "14:30".
/// Retorna (epoch ms, tem hora). Sem hora, guarda ao meio-dia local.
pub fn parse_due(text: &str, today: NaiveDate) -> Option<(i64, bool)> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    let mut parts = text.split_whitespace();
    let date_part = parts.next()?;
    let time_part = parts.next();

    let day = match date_part {
        "hoje" => Some(today),
        "amanhã" | "amanha" => today.succ_opt(),
        "ontem" => today.pred_opt(),
        p if p.starts_with('+') => p[1..].parse::<u64>().ok().and_then(|n| today.checked_add_days(chrono::Days::new(n))),
        p => {
            let weekday = ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"]
                .iter()
                .position(|w| p.starts_with(w) || (*w == "sáb" && p.starts_with("sab")));
            if let Some(target) = weekday {
                // Próxima ocorrência (hoje conta se for o mesmo dia).
                let current = today.weekday().num_days_from_monday() as i64;
                let ahead = (target as i64 - current).rem_euclid(7) as u64;
                today.checked_add_days(chrono::Days::new(ahead))
            } else {
                NaiveDate::parse_from_str(p, "%Y-%m-%d")
                    .or_else(|_| NaiveDate::parse_from_str(p, "%d/%m/%Y"))
                    .or_else(|_| NaiveDate::parse_from_str(&format!("{p}/{}", today.year()), "%d/%m/%Y"))
                    .ok()
            }
        }
    }?;

    match time_part {
        Some(t) => {
            let (h, m) = t.split_once(':').unwrap_or((t.trim_end_matches('h'), "0"));
            let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
            (h < 24 && m < 60).then(|| (local_ms(day, h, m), true))
        }
        None => Some((local_ms(day, 12, 0), false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Priority;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, day).unwrap()
    }

    fn task(id: i64, due: Option<NaiveDate>, priority: Priority) -> Task {
        Task {
            id,
            due_at: due.map(|day| local_ms(day, 12, 0)),
            priority,
            ..Task::new(&format!("t{id}"))
        }
    }

    #[test]
    fn prazos_digitados() {
        let today = d(15); // quarta
        assert_eq!(parse_due("hoje", today), Some((local_ms(d(15), 12, 0), false)));
        assert_eq!(parse_due("Amanhã 14:30", today), Some((local_ms(d(16), 14, 30), true)));
        assert_eq!(parse_due("+3", today).map(|x| local_date(x.0)), Some(d(18)));
        assert_eq!(parse_due("sex", today).map(|x| local_date(x.0)), Some(d(17)));
        assert_eq!(parse_due("qua", today).map(|x| local_date(x.0)), Some(d(15)));
        assert_eq!(parse_due("seg", today).map(|x| local_date(x.0)), Some(d(20)));
        assert_eq!(parse_due("2025-02-01", today).map(|x| local_date(x.0)), NaiveDate::from_ymd_opt(2025, 2, 1));
        assert_eq!(parse_due("12/03", today).map(|x| local_date(x.0)), NaiveDate::from_ymd_opt(2025, 3, 12));
        assert_eq!(parse_due("12/03/2026 9h", today), Some((local_ms(NaiveDate::from_ymd_opt(2026, 3, 12).unwrap(), 9, 0), true)));
        assert_eq!(parse_due("ontem 25:00", today), None);
        assert_eq!(parse_due("talvez", today), None);
        assert_eq!(parse_due("", today), None);
    }

    #[test]
    fn visoes_e_ordenacao() {
        let today = d(15);
        let now = local_ms(today, 10, 0);
        let mut done = task(5, Some(d(10)), Priority::Unset);
        done.done_at = Some(now);
        let mut in_list = task(6, None, Priority::Unset);
        in_list.list_id = Some(1);
        let tasks = vec![
            task(1, Some(d(14)), Priority::Unset), // atrasada
            task(2, Some(d(15)), Priority::Low),
            task(3, Some(d(15)), Priority::High),
            task(4, Some(d(20)), Priority::Unset),
            done,
            in_list,
            task(7, None, Priority::Medium), // caixa de entrada, sem prazo
        ];
        let ids = |v: Vec<&Task>| v.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(filter_view(&tasks, TaskView::Today, today, now)), vec![1, 3, 2]);
        assert_eq!(ids(filter_view(&tasks, TaskView::Next7, today, now)), vec![3, 2, 4]);
        assert_eq!(ids(filter_view(&tasks, TaskView::Overdue, today, now)), vec![1]);
        assert_eq!(ids(filter_view(&tasks, TaskView::Inbox, today, now)), vec![1, 3, 2, 4, 7]);
        // Sem prazo: prioridade maior primeiro (7 é média, 6 não tem).
        assert_eq!(ids(filter_view(&tasks, TaskView::All, today, now)), vec![1, 3, 2, 4, 7, 6]);
        assert_eq!(ids(filter_view(&tasks, TaskView::Done, today, now)), vec![5]);
    }

    #[test]
    fn atraso_com_hora() {
        let today = d(15);
        let mut t = Task::new("reunião");
        t.due_at = Some(local_ms(today, 9, 0));
        t.due_has_time = true;
        assert!(is_overdue(&t, today, local_ms(today, 10, 0)));
        assert!(!is_overdue(&t, today, local_ms(today, 8, 0)));
        t.due_has_time = false;
        assert!(!is_overdue(&t, today, local_ms(today, 23, 0)), "sem hora, só atrasa no dia seguinte");
    }

    #[test]
    fn rotulos_de_prazo() {
        let today = d(15);
        let at = |day: NaiveDate| Task { due_at: Some(local_ms(day, 12, 0)), ..Task::new("x") };
        assert_eq!(due_label(&at(d(15)), today).as_deref(), Some("Hoje"));
        assert_eq!(due_label(&at(d(16)), today).as_deref(), Some("Amanhã"));
        assert_eq!(due_label(&at(d(14)), today).as_deref(), Some("Ontem"));
        assert_eq!(due_label(&at(d(17)), today).as_deref(), Some("Sex"));
        assert_eq!(due_label(&at(d(30)), today).as_deref(), Some("30/01"));
        let mut timed = at(d(16));
        timed.due_at = Some(local_ms(d(16), 14, 30));
        timed.due_has_time = true;
        assert_eq!(due_label(&timed, today).as_deref(), Some("Amanhã 14:30"));
        assert_eq!(due_label(&Task::new("sem prazo"), today), None);
        assert_eq!(TaskView::from_key("week"), Some(TaskView::Next7));
    }
}
