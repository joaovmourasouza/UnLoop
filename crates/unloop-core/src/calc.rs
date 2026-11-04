//! Regras de negócio puras (sem I/O). Ver docs/REA-Quitzilla.md §3.5.

use std::collections::BTreeMap;

use crate::model::{DiaryEntry, Goal, Habit, HabitRule, Reward, TimeInvestment, Trophy};
use crate::units::{MS_PER_DAY, MS_PER_MINUTE};

/// Início da sequência atual: a recaída mais recente, ou o início da jornada.
/// Recaídas anteriores ao início da jornada são ignoradas.
pub fn streak_start(quit_at: i64, entries: &[DiaryEntry]) -> i64 {
    entries
        .iter()
        .filter(|e| e.is_reset)
        .map(|e| e.occurred_at)
        .fold(quit_at, i64::max)
}

/// Se uma entrada reinicia a sequência: a regra escolhida decide
/// (não permitido = reinicia, permitido = não reinicia); sem regra, vale a escolha manual.
pub fn entry_resets(rule: Option<&HabitRule>, manual: bool) -> bool {
    rule.map_or(manual, |r| r.kind.resets_streak())
}

/// Timestamps das recaídas, em ordem crescente.
pub fn reset_timestamps(entries: &[DiaryEntry]) -> Vec<i64> {
    let mut v: Vec<i64> = entries.iter().filter(|e| e.is_reset).map(|e| e.occurred_at).collect();
    v.sort_unstable();
    v
}

/// Períodos de abstinência já encerrados (o período atual NÃO entra).
pub fn abstinence_periods(quit_at: i64, sorted_resets: &[i64]) -> Vec<i64> {
    let mut periods = Vec::with_capacity(sorted_resets.len());
    let mut prev = quit_at;
    for &r in sorted_resets.iter().filter(|&&r| r >= quit_at) {
        periods.push(r - prev);
        prev = r;
    }
    periods
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeriodStats {
    pub periods: Vec<i64>,
}

impl PeriodStats {
    pub fn count(&self) -> usize {
        self.periods.len()
    }
    pub fn longest(&self) -> Option<i64> {
        self.periods.iter().copied().max()
    }
    pub fn shortest(&self) -> Option<i64> {
        self.periods.iter().copied().min()
    }
    pub fn average(&self) -> Option<i64> {
        if self.periods.is_empty() {
            None
        } else {
            Some(self.periods.iter().sum::<i64>() / self.periods.len() as i64)
        }
    }
    pub fn previous(&self) -> Option<i64> {
        self.periods.last().copied()
    }
}

/// Momento em que cada troféu foi conquistado (`None` = ainda não).
///
/// Pontos = [quit_at, recaídas…, now]. O troféu T é conquistado no primeiro
/// intervalo consecutivo >= T.ms, no instante `início do intervalo + T.ms`.
pub fn compute_trophies(quit_at: i64, sorted_resets: &[i64], now: i64) -> BTreeMap<i64, Option<i64>> {
    let mut result = BTreeMap::new();
    if now < quit_at {
        for t in Trophy::ALL {
            result.insert(t.code(), None);
        }
        return result;
    }
    let mut points = Vec::with_capacity(sorted_resets.len() + 2);
    points.push(quit_at);
    points.extend(sorted_resets.iter().copied().filter(|&r| r >= quit_at && r <= now));
    points.push(now);

    for &t in Trophy::ALL {
        let at = if t == Trophy::DecisionMade {
            Some(quit_at)
        } else {
            points
                .windows(2)
                .find(|w| w[1] - w[0] >= t.ms())
                .map(|w| w[0] + t.ms())
        };
        result.insert(t.code(), at);
    }
    result
}

// ------------------------------------------------------------------ Metas

pub fn goal_target(g: &Goal, start: i64) -> i64 {
    if g.is_personal {
        g.target_at.unwrap_or(start)
    } else {
        start + g.duration_ms.unwrap_or(0)
    }
}

pub fn goal_reached(g: &Goal, start: i64, now: i64) -> bool {
    now >= goal_target(g, start)
}

pub fn goal_progress(g: &Goal, start: i64, now: i64) -> f64 {
    let span = (goal_target(g, start) - start).max(1);
    ((now - start) as f64 / span as f64).clamp(0.0, 1.0)
}

pub fn goal_remaining(g: &Goal, start: i64, now: i64) -> i64 {
    (goal_target(g, start) - now).max(0)
}

pub fn sort_goals_by_target(goals: &[Goal], start: i64) -> Vec<Goal> {
    let mut list = goals.to_vec();
    list.sort_by_key(|g| (goal_target(g, start), g.id));
    list
}

/// Meta atual: a fixada (se ainda não alcançada); senão a próxima não alcançada.
pub fn current_goal(goals: &[Goal], start: i64, now: i64) -> Option<Goal> {
    if let Some(forced) = goals.iter().find(|g| g.is_forced_current && !goal_reached(g, start, now)) {
        return Some(forced.clone());
    }
    sort_goals_by_target(goals, start)
        .into_iter()
        .find(|g| !goal_reached(g, start, now))
}

// ----------------------------------------------------------------- Saldos

#[derive(Clone, Debug, PartialEq)]
pub struct Balances {
    /// Economia bruta desde o início da jornada.
    pub saved_total: f64,
    /// Economia desde a última recaída.
    pub saved_since_start: f64,
    /// Dinheiro registrado no diário.
    pub spent_on_habit: f64,
    /// Preço das recompensas compradas (inclusive arquivadas).
    pub rewards_spent: f64,
    pub time_total_ms: i64,
    pub time_since_start_ms: i64,
    pub time_lost_ms: i64,
    pub invested_ms: i64,
    pub money_per_ms: f64,
    pub time_per_ms: f64,
}

impl Balances {
    pub fn net_saved(&self) -> f64 {
        self.saved_total - self.spent_on_habit
    }
    pub fn money_balance(&self) -> f64 {
        self.net_saved() - self.rewards_spent
    }
    pub fn net_time_ms(&self) -> i64 {
        self.time_total_ms - self.time_lost_ms
    }
    pub fn time_balance_ms(&self) -> i64 {
        self.net_time_ms() - self.invested_ms
    }
}

pub fn compute_balances(
    habit: &Habit,
    entries: &[DiaryEntry],
    rewards: &[Reward],
    investments: &[TimeInvestment],
    start: i64,
    now: i64,
) -> Balances {
    let since_quit = (now - habit.quit_at).max(0);
    let since_start = (now - start).max(0);
    let money = |ms: i64| ms as f64 * habit.daily_money / MS_PER_DAY as f64;
    let time = |ms: i64| (ms as f64 * habit.daily_minutes * MS_PER_MINUTE as f64 / MS_PER_DAY as f64).round() as i64;

    Balances {
        saved_total: money(since_quit),
        saved_since_start: money(since_start),
        spent_on_habit: entries.iter().map(|e| e.money_spent).sum(),
        rewards_spent: rewards.iter().filter(|r| r.is_purchased()).map(|r| r.price).sum(),
        time_total_ms: time(since_quit),
        time_since_start_ms: time(since_start),
        time_lost_ms: entries.iter().map(|e| e.time_spent_ms).sum(),
        invested_ms: investments.iter().map(|i| i.invested_ms).sum(),
        money_per_ms: habit.daily_money / MS_PER_DAY as f64,
        time_per_ms: habit.daily_minutes * MS_PER_MINUTE as f64 / MS_PER_DAY as f64,
    }
}

/// Tempo (ms) até o saldo cobrir `price`. `Some(0)` se já cobre; `None` se nunca.
pub fn time_to_afford(price: f64, balance: f64, money_per_ms: f64) -> Option<i64> {
    if balance >= price {
        Some(0)
    } else if money_per_ms <= 0.0 {
        None
    } else {
        Some(((price - balance) / money_per_ms).ceil() as i64)
    }
}

// --------------------------------------------------------------- Snapshot

/// Todos os dados de um hábito (carregados do banco).
#[derive(Clone, Debug)]
pub struct HabitBundle {
    pub habit: Habit,
    pub entries: Vec<DiaryEntry>,
    pub goals: Vec<Goal>,
    pub rewards: Vec<Reward>,
    pub investments: Vec<TimeInvestment>,
    pub reasons: Vec<crate::model::TextItem>,
    pub affirmations: Vec<crate::model::TextItem>,
    pub rules: Vec<HabitRule>,
    /// Progresso por dia de hábitos de check diário.
    pub checkins: Vec<crate::model::DayProgress>,
}

/// Tudo que a UI precisa sobre um hábito num instante.
#[derive(Clone, Debug)]
pub struct HabitSnapshot {
    pub now: i64,
    pub start: i64,
    pub goal: Option<Goal>,
    pub periods: PeriodStats,
    /// Código do troféu → momento da conquista.
    pub trophies: BTreeMap<i64, Option<i64>>,
    pub balances: Balances,
}

impl HabitSnapshot {
    pub fn elapsed(&self) -> i64 {
        (self.now - self.start).max(0)
    }
    pub fn goal_progress(&self) -> f64 {
        self.goal.as_ref().map_or(1.0, |g| goal_progress(g, self.start, self.now))
    }
    pub fn goal_remaining(&self) -> i64 {
        self.goal.as_ref().map_or(0, |g| goal_remaining(g, self.start, self.now))
    }
    pub fn goal_target(&self) -> i64 {
        self.goal.as_ref().map_or(self.now, |g| goal_target(g, self.start))
    }
    pub fn trophy_at(&self, t: Trophy) -> Option<i64> {
        self.trophies.get(&t.code()).copied().flatten()
    }
    pub fn trophies_achieved(&self) -> usize {
        self.trophies.values().filter(|v| v.is_some()).count()
    }
}

impl HabitBundle {
    pub fn rule(&self, id: Option<i64>) -> Option<&HabitRule> {
        let id = id?;
        self.rules.iter().find(|r| r.id == id)
    }

    /// Quantas entradas do diário citam a regra.
    pub fn rule_usage(&self, rule_id: i64) -> usize {
        self.entries.iter().filter(|e| e.rule_id == Some(rule_id)).count()
    }

    /// Estatísticas de check diário até `today`.
    pub fn checkin_stats(&self, today: crate::checkin::NaiveDate) -> crate::checkin::CheckinStats {
        crate::checkin::checkin_stats(
            crate::checkin::local_date(self.habit.quit_at),
            self.habit.schedule_days,
            &crate::checkin::done_days(&self.checkins, self.habit.daily_target),
            today,
        )
    }

    /// Progresso registrado num dia.
    pub fn amount_on(&self, day: crate::checkin::NaiveDate) -> f64 {
        crate::checkin::amount_on(&self.checkins, day)
    }

    pub fn snapshot(&self, now: i64) -> HabitSnapshot {
        let start = streak_start(self.habit.quit_at, &self.entries);
        let resets = reset_timestamps(&self.entries);
        HabitSnapshot {
            now,
            start,
            goal: current_goal(&self.goals, start, now),
            periods: PeriodStats { periods: abstinence_periods(self.habit.quit_at, &resets) },
            trophies: compute_trophies(self.habit.quit_at, &resets, now),
            balances: compute_balances(&self.habit, &self.entries, &self.rewards, &self.investments, start, now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{MS_PER_HOUR, MS_PER_MINUTE};

    const BASE: i64 = 1_735_689_600_000; // 2025-01-01T00:00:00Z

    fn day(n: f64) -> i64 {
        BASE + (n * MS_PER_DAY as f64).round() as i64
    }

    fn habit() -> Habit {
        let mut h = Habit::new("Teste", BASE);
        h.id = 1;
        h.daily_money = 10.0;
        h.daily_minutes = 60.0;
        h
    }

    fn entries() -> Vec<DiaryEntry> {
        vec![
            DiaryEntry { money_spent: 5.0, ..DiaryEntry::reset(1, day(2.0)) },
            DiaryEntry { money_spent: 7.0, time_spent_ms: 30 * MS_PER_MINUTE, ..DiaryEntry::reset(1, day(12.0)) },
            DiaryEntry { is_reset: false, note: Some("só uma nota".into()), ..DiaryEntry::reset(1, day(13.0)) },
        ]
    }

    fn goal(id: i64, days: i64, forced: bool) -> Goal {
        Goal {
            id,
            is_forced_current: forced,
            ..Goal::preset(1, &format!("{days} dias"), "K", days * MS_PER_DAY, "FF0000")
        }
    }

    #[test]
    fn sequencia_comeca_na_ultima_recaida() {
        assert_eq!(streak_start(BASE, &entries()), day(12.0));
        assert_eq!(streak_start(BASE, &[]), BASE);
        assert_eq!(streak_start(BASE, &[DiaryEntry::reset(1, BASE - MS_PER_DAY)]), BASE);
        assert_eq!(streak_start(BASE, &entries()[2..]), BASE);
    }

    #[test]
    fn regras_decidem_se_reinicia() {
        use crate::model::RuleKind;
        let rule = |kind| HabitRule { id: 1, habit_id: 1, kind, text: "x".into(), position: 0 };
        assert!(!entry_resets(Some(&rule(RuleKind::Allowed)), true));
        assert!(entry_resets(Some(&rule(RuleKind::Forbidden)), false));
        assert!(entry_resets(None, true));
        assert!(!entry_resets(None, false));
    }

    #[test]
    fn periodos_e_estatisticas() {
        let s = PeriodStats { periods: abstinence_periods(BASE, &reset_timestamps(&entries())) };
        assert_eq!(s.periods, vec![2 * MS_PER_DAY, 10 * MS_PER_DAY]);
        assert_eq!(s.count(), 2);
        assert_eq!(s.longest(), Some(10 * MS_PER_DAY));
        assert_eq!(s.shortest(), Some(2 * MS_PER_DAY));
        assert_eq!(s.average(), Some(6 * MS_PER_DAY));
        assert_eq!(s.previous(), Some(10 * MS_PER_DAY));
        assert_eq!(PeriodStats { periods: vec![] }.average(), None);
    }

    #[test]
    fn trofeus() {
        let t = compute_trophies(BASE, &reset_timestamps(&entries()), day(15.0));
        let at = |tr: Trophy| t[&tr.code()];
        assert_eq!(at(Trophy::DecisionMade), Some(BASE));
        assert_eq!(at(Trophy::Day1), Some(day(1.0)));
        assert_eq!(at(Trophy::Day3), Some(day(5.0)));
        assert_eq!(at(Trophy::Day7), Some(day(9.0)));
        assert_eq!(at(Trophy::Day10), Some(day(12.0)));
        assert_eq!(at(Trophy::Day14), None);

        // A sequência atual também conta; recaída não remove troféu.
        assert_eq!(compute_trophies(BASE, &[], day(31.0))[&Trophy::Day30.code()], Some(day(30.0)));
        assert_eq!(compute_trophies(BASE, &[day(8.0)], day(9.0))[&Trophy::Day7.code()], Some(day(7.0)));
        // Início no futuro: nada.
        assert!(compute_trophies(day(10.0), &[], day(5.0)).values().all(|v| v.is_none()));
    }

    #[test]
    fn metas() {
        let start = day(12.0);
        let now = day(15.0);
        let g = |id, d| goal(id, d, false);
        assert_eq!(current_goal(&[g(1, 1), g(2, 3), g(3, 7)], start, now).map(|x| x.id), Some(3));
        assert_eq!(
            current_goal(&[g(1, 1), g(3, 7), goal(4, 30, true)], start, now).map(|x| x.id),
            Some(4)
        );
        assert_eq!(current_goal(&[goal(1, 1, true), g(3, 7)], start, now).map(|x| x.id), Some(3));
        assert_eq!(current_goal(&[g(1, 1), g(2, 2)], start, now), None);
        assert!((goal_progress(&g(3, 7), start, now) - 3.0 / 7.0).abs() < 1e-9);
        assert_eq!(goal_remaining(&g(3, 7), start, now), 4 * MS_PER_DAY);

        let personal = Goal::personal(1, "Viagem", day(20.0), "00FF00");
        assert_eq!(goal_target(&personal, start), day(20.0));
        assert_eq!(goal_remaining(&personal, start, now), 5 * MS_PER_DAY);
        assert!(goal_reached(&personal, start, day(20.0)));
    }

    #[test]
    fn saldos() {
        let rewards = vec![
            Reward { purchased_at: Some(day(14.0)), ..Reward::new(1, "Tênis", 20.0) },
            Reward::new(1, "Viagem", 100.0),
            Reward { purchased_at: Some(day(14.0)), archived: true, ..Reward::new(1, "Arquivada", 5.0) },
        ];
        let investments = vec![TimeInvestment { invested_ms: 60 * MS_PER_MINUTE, ..TimeInvestment::new(1, "Leitura", 30 * MS_PER_MINUTE) }];
        let b = compute_balances(&habit(), &entries(), &rewards, &investments, day(12.0), day(15.0));

        assert!((b.saved_total - 150.0).abs() < 1e-9);
        assert!((b.saved_since_start - 30.0).abs() < 1e-9);
        assert!((b.spent_on_habit - 12.0).abs() < 1e-9);
        assert!((b.rewards_spent - 25.0).abs() < 1e-9);
        assert!((b.money_balance() - 113.0).abs() < 1e-9);

        assert_eq!(b.time_total_ms, 15 * MS_PER_HOUR);
        assert_eq!(b.time_since_start_ms, 3 * MS_PER_HOUR);
        assert_eq!(b.time_lost_ms, 30 * MS_PER_MINUTE);
        assert_eq!(b.time_balance_ms(), 15 * MS_PER_HOUR - 90 * MS_PER_MINUTE);

        assert_eq!(time_to_afford(100.0, 113.0, b.money_per_ms), Some(0));
        let wait = time_to_afford(200.0, 113.0, b.money_per_ms).unwrap();
        assert!((wait - 87 * MS_PER_DAY / 10).abs() <= 1);
        assert_eq!(time_to_afford(200.0, 113.0, 0.0), None);
    }

    #[test]
    fn snapshot() {
        let bundle = HabitBundle {
            habit: habit(),
            entries: entries(),
            goals: vec![goal(1, 7, false)],
            rewards: vec![],
            investments: vec![],
            reasons: vec![],
            affirmations: vec![],
            rules: vec![],
            checkins: vec![],
        };
        let s = bundle.snapshot(day(15.0));
        assert_eq!(s.start, day(12.0));
        assert_eq!(s.elapsed(), 3 * MS_PER_DAY);
        assert_eq!(s.goal.as_ref().map(|g| g.name.as_str()), Some("7 dias"));
        assert_eq!(s.trophies_achieved(), 5);
        assert_eq!(s.trophy_at(Trophy::Day7), Some(day(9.0)));
    }
}
