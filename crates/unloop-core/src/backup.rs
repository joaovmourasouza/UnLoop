//! Backup JSON no mesmo formato do backup em arquivo do Quitzilla 3.0.2
//! (`BackupData`), com campos extras do UnLoop. Um único importador lê os dois.

use std::fmt;

use serde_json::{json, Map, Value};

use crate::db::{Db, TextTable};
use crate::format::format_compact;
use crate::model::*;
use crate::presets::{preset_by_key, preset_for_duration};

pub const FORMAT_ID: &str = "unloop";
pub const FORMAT_VERSION: i64 = 1;

#[derive(Debug)]
pub enum BackupError {
    Format(String),
    Db(rusqlite::Error),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::Format(m) => write!(f, "{m}"),
            BackupError::Db(e) => write!(f, "erro no banco de dados: {e}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<rusqlite::Error> for BackupError {
    fn from(e: rusqlite::Error) -> Self {
        BackupError::Db(e)
    }
}

pub fn export_backup(db: &Db, app_version: &str) -> rusqlite::Result<Value> {
    let habits = db.habits()?;
    let (mut entries, mut goals, mut rewards, mut investments, mut reasons, mut affirmations) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut rules = vec![];
    let mut checkins = vec![];

    for h in &habits {
        for p in db.checkins(h.id)? {
            checkins.push(json!({ "habitId": h.id, "day": p.day, "amount": p.amount }));
        }
        for r in db.rules(h.id)? {
            rules.push(json!({
                "id": r.id, "habitId": h.id, "kind": r.kind.legacy(),
                "text": r.text, "listPosition": r.position,
            }));
        }
        for e in db.entries(h.id)? {
            entries.push(json!({
                "id": e.id, "habitId": h.id, "timestamp": e.occurred_at,
                "moneySpent": e.money_spent, "timeSpent": e.time_spent_ms,
                "isReset": e.is_reset, "comment": e.note, "ruleId": e.rule_id,
            }));
        }
        for g in db.goals(h.id)? {
            goals.push(json!({
                "id": g.id, "habitId": h.id, "name": g.name,
                // Como no Quitzilla: em meta pessoal, durationMs guarda a data-alvo.
                "durationMs": if g.is_personal { g.target_at } else { g.duration_ms },
                "hexColor": format!("#{}", g.hex_color),
                "isPersonal": g.is_personal, "isForceCurrent": g.is_forced_current,
                "presetKey": g.preset_key,
            }));
        }
        for r in db.rewards(h.id)? {
            rewards.push(json!({
                "id": r.id, "habitId": h.id, "name": r.name, "price": r.price,
                "isPurchased": r.is_purchased(), "isDeleted": r.archived,
                "listPosition": r.position, "purchaseDate": r.purchased_at,
            }));
        }
        for t in db.investments(h.id)? {
            investments.push(json!({
                "id": t.id, "habitId": h.id, "name": t.name,
                "sessionDuration": t.session_ms, "totalInvestments": t.invested_ms,
                "isDeleted": t.archived, "listPosition": t.position,
            }));
        }
        for r in db.text_items(TextTable::Reason, h.id)? {
            reasons.push(json!({ "id": r.id, "habitId": h.id, "text": r.text, "listPosition": r.position }));
        }
        for a in db.text_items(TextTable::Affirmation, h.id)? {
            affirmations.push(json!({ "id": a.id, "habitId": h.id, "text": a.text, "listPosition": a.position }));
        }
    }

    let mut focus_sessions: Vec<Value> = Vec::new();
    for f in db.focus_sessions_since(0)? {
        focus_sessions.push(json!({
            "habitId": f.habit_id, "startedAt": f.started_at, "endedAt": f.ended_at,
            "durationMs": f.duration_ms, "mode": f.mode.code(),
        }));
    }

    let habits_json: Vec<Value> = habits
        .iter()
        .map(|h| {
            json!({
                "id": h.id, "name": h.name,
                "icon": icon(h.icon).legacy, "color": color(h.color).legacy,
                "quitDate": h.quit_at,
                "dailyMoneySpending": h.daily_money, "dailyTimeSpending": h.daily_minutes,
                "dailyFrequency": h.daily_frequency, "listPosition": h.position,
                "preferredSpendingType": h.preferred_metric.legacy(),
                "counterType": h.counter_type.legacy(), "counterStyle": h.counter_style.legacy(),
                "habitSinceMs": h.habit_since_ms, "createdAt": h.created_at,
                "kind": h.kind.legacy(), "scheduleDays": h.schedule_days,
                "dailyTarget": h.daily_target, "targetUnit": h.target_unit,
            })
        })
        .collect();

    Ok(json!({
        "format": FORMAT_ID,
        "version": FORMAT_VERSION,
        "appVersion": app_version,
        "createdAt": crate::now_ms(),
        "habits": habits_json,
        "diaryEntries": entries,
        "goals": goals,
        "rewards": rewards,
        "timeInvestments": investments,
        "reasons": reasons,
        "affirmations": affirmations,
        "rules": rules,
        "checkins": checkins,
        "focusSessions": focus_sessions,
        "settings": {
            "currencyCode": db.setting("currency")?,
            "mainTitle": db.setting("motto")?,
            "theme": db.setting("theme")?,
        },
    }))
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ImportSummary {
    pub habits: usize,
    pub entries: usize,
    pub goals: usize,
    pub rewards: usize,
    pub investments: usize,
    pub reasons: usize,
    pub affirmations: usize,
    pub rules: usize,
    pub checkins: usize,
    pub focus_sessions: usize,
    pub from_quitzilla: bool,
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} hábito(s), {} check(s), {} sessão(ões) de foco, {} regra(s), {} entrada(s) no diário, {} meta(s), {} recompensa(s), \
             {} investimento(s), {} motivo(s), {} afirmação(ões)",
            self.habits,
            self.checkins,
            self.focus_sessions,
            self.rules,
            self.entries,
            self.goals,
            self.rewards,
            self.investments,
            self.reasons,
            self.affirmations
        )
    }
}

/// Importa um backup do UnLoop ou do Quitzilla numa transação.
/// Com `replace_existing`, apaga todos os hábitos atuais antes.
pub fn import_backup(db: &Db, json: &Value, replace_existing: bool) -> Result<ImportSummary, BackupError> {
    let habits = sorted_by_position(list(json, "habits"));
    if habits.is_empty() {
        return Err(BackupError::Format("Nenhum hábito encontrado no arquivo.".into()));
    }
    let now = crate::now_ms();
    let mut summary = ImportSummary {
        from_quitzilla: json.get("format").and_then(Value::as_str) != Some(FORMAT_ID),
        ..Default::default()
    };

    db.transaction(|| {
        if replace_existing {
            db.delete_all_habits()?;
        }
        let mut id_map = std::collections::HashMap::<i64, i64>::new();
        let mut build_ids = std::collections::HashSet::<i64>::new();

        for m in &habits {
            let name = text(m, "name");
            let mut h = Habit::new(if name.is_empty() { "Hábito".to_string() } else { name }, int(m, "quitDate").unwrap_or(now));
            h.icon = match m.get("icon") {
                Some(Value::String(s)) => icon_by_legacy(s).code,
                Some(v) => v.as_i64().map(|c| icon(c).code).unwrap_or(ICONS[0].code),
                None => ICONS[0].code,
            };
            h.color = match m.get("color") {
                Some(Value::String(s)) => color_by_legacy(s).code,
                Some(v) => v.as_i64().map(|c| color(c).code).unwrap_or(DEFAULT_COLOR),
                None => DEFAULT_COLOR,
            };
            h.daily_money = non_neg(m.get("dailyMoneySpending").or_else(|| m.get("dailySpending")));
            h.daily_minutes = non_neg(m.get("dailyTimeSpending"));
            h.daily_frequency = non_neg(m.get("dailyFrequency"));
            h.habit_since_ms = int(m, "habitSinceMs");
            h.preferred_metric = coded(m.get("preferredSpendingType"), MetricType::from_code, MetricType::from_legacy);
            h.counter_type = coded(m.get("counterType"), CounterType::from_code, CounterType::from_legacy);
            h.counter_style = coded(m.get("counterStyle"), CounterStyle::from_code, CounterStyle::from_legacy);
            h.created_at = int(m, "createdAt").unwrap_or(now);
            h.kind = coded(m.get("kind"), HabitKind::from_code, HabitKind::from_legacy);
            h.schedule_days = crate::checkin::normalize_mask(int(m, "scheduleDays").unwrap_or(127));
            h.daily_target = non_neg(m.get("dailyTarget"));
            h.target_unit = text(m, "targetUnit").trim().to_string();

            let new_id = db.insert_habit(&h, false)?;
            id_map.insert(int(m, "id").unwrap_or(0), new_id);
            if h.is_build() {
                build_ids.insert(new_id);
            }
            summary.habits += 1;
        }
        let map_habit = |v: &Value| int(v, "habitId").and_then(|old| id_map.get(&old).copied());

        for c in list(json, "checkins") {
            let Some(hid) = map_habit(c) else { continue };
            let Some(day) = c.get("day").and_then(Value::as_str).and_then(crate::checkin::parse_day) else {
                continue;
            };
            let amount = match c.get("amount") {
                Some(v) if !v.is_null() => non_neg(Some(v)),
                _ => 1.0,
            };
            db.set_progress(hid, &crate::checkin::day_key(day), amount)?;
            summary.checkins += 1;
        }

        for f in list(json, "focusSessions") {
            let duration = int(f, "durationMs").unwrap_or(0);
            let started = int(f, "startedAt").unwrap_or(0);
            if duration <= 0 || started <= 0 {
                continue;
            }
            db.insert_focus(&crate::pomodoro::FocusRecord {
                id: 0,
                habit_id: map_habit(f),
                started_at: started,
                ended_at: int(f, "endedAt").unwrap_or(started + duration),
                duration_ms: duration,
                mode: crate::pomodoro::Mode::from_code(int(f, "mode").unwrap_or(0)),
            })?;
            summary.focus_sessions += 1;
        }

        // Regras antes do diário, para remapear `ruleId`.
        let mut rule_map = std::collections::HashMap::<i64, (i64, RuleKind)>::new();
        for r in sorted_by_position(list(json, "rules")) {
            let Some(hid) = map_habit(r) else { continue };
            let body = text(r, "text");
            if body.trim().is_empty() {
                continue;
            }
            let kind = coded(r.get("kind"), RuleKind::from_code, RuleKind::from_legacy);
            let new_id = db.insert_rule(hid, kind, &body)?;
            rule_map.insert(int(r, "id").unwrap_or(0), (new_id, kind));
            summary.rules += 1;
        }

        for e in list(json, "diaryEntries") {
            let Some(hid) = map_habit(e) else { continue };
            let rule = int(e, "ruleId").and_then(|old| rule_map.get(&old).copied());
            let manual = boolean(e.get("isReset"), true);
            db.insert_entry(&DiaryEntry {
                id: 0,
                habit_id: hid,
                occurred_at: int(e, "timestamp").unwrap_or(now),
                money_spent: non_neg(e.get("moneySpent")),
                time_spent_ms: non_neg(e.get("timeSpent")).round() as i64,
                is_reset: rule.map_or(manual, |(_, kind)| kind.resets_streak()),
                note: e.get("comment").and_then(Value::as_str).map(str::to_string),
                rule_id: rule.map(|(id, _)| id),
            })?;
            summary.entries += 1;
        }

        let mut with_goals = std::collections::HashSet::new();
        let mut with_forced = std::collections::HashSet::new();
        for g in list(json, "goals") {
            let Some(hid) = map_habit(g) else { continue };
            let personal = boolean(g.get("isPersonal"), false);
            let ms = int(g, "durationMs").unwrap_or(0);
            let preset = if personal {
                None
            } else {
                g.get("presetKey")
                    .and_then(Value::as_str)
                    .and_then(preset_by_key)
                    .or_else(|| preset_for_duration(ms))
            };
            let forced = boolean(g.get("isForceCurrent"), false) && with_forced.insert(hid);
            let name = text(g, "name");
            let fallback_hex = preset.map(|p| p.hex.as_str()).unwrap_or("757575");
            db.insert_goal(&Goal {
                id: 0,
                habit_id: hid,
                name: match preset {
                    Some(p) => p.label.clone(),
                    None if name.is_empty() => format_compact(ms),
                    None => name,
                },
                preset_key: preset.map(|p| p.key.clone()),
                duration_ms: if personal { None } else { Some(ms) },
                target_at: if personal { Some(ms) } else { None },
                hex_color: normalize_hex(&text(g, "hexColor"), fallback_hex),
                is_personal: personal,
                is_forced_current: forced,
            })?;
            with_goals.insert(hid);
            summary.goals += 1;
        }
        for hid in id_map.values() {
            if !with_goals.contains(hid) && !build_ids.contains(hid) {
                db.seed_preset_goals(*hid)?;
            }
        }

        for r in sorted_by_position(list(json, "rewards")) {
            let Some(hid) = map_habit(r) else { continue };
            let purchased = boolean(r.get("isPurchased"), false);
            let name = if r.get("name").is_some() { text(r, "name") } else { text(r, "text") };
            db.insert_reward(&Reward {
                purchased_at: if purchased { Some(int(r, "purchaseDate").unwrap_or(now)) } else { None },
                archived: boolean(r.get("isDeleted"), false),
                ..Reward::new(hid, &name, non_neg(r.get("price")))
            })?;
            summary.rewards += 1;
        }

        for t in sorted_by_position(list(json, "timeInvestments")) {
            let Some(hid) = map_habit(t) else { continue };
            let session = int(t, "sessionDuration").or_else(|| int(t, "singleSessionDuration")).unwrap_or(0);
            if session <= 0 {
                continue;
            }
            let name = if t.get("name").is_some() { text(t, "name") } else { text(t, "investmentName") };
            db.insert_investment(&TimeInvestment {
                invested_ms: int(t, "totalInvestments").unwrap_or(0).max(0),
                archived: boolean(t.get("isDeleted"), false),
                ..TimeInvestment::new(hid, &name, session)
            })?;
            summary.investments += 1;
        }

        for (key, table) in [("reasons", TextTable::Reason), ("affirmations", TextTable::Affirmation)] {
            for item in sorted_by_position(list(json, key)) {
                let Some(hid) = map_habit(item) else { continue };
                let body = text(item, "text");
                if body.trim().is_empty() {
                    continue;
                }
                db.insert_text_item(table, hid, body.trim())?;
                match table {
                    TextTable::Reason => summary.reasons += 1,
                    TextTable::Affirmation => summary.affirmations += 1,
                }
            }
        }

        if let Some(settings) = json.get("settings") {
            if let Some(c) = settings.get("currencyCode").and_then(Value::as_str) {
                if c.len() == 3 && c.chars().all(|ch| ch.is_ascii_uppercase()) {
                    db.set_setting("currency", c)?;
                }
            }
            if let Some(m) = settings.get("mainTitle").and_then(Value::as_str) {
                if !m.trim().is_empty() {
                    db.set_setting("motto", m.trim())?;
                }
            }
            if let Some(t) = settings.get("theme").and_then(Value::as_str) {
                if ["system", "light", "dark"].contains(&t) {
                    db.set_setting("theme", t)?;
                }
            }
        }
        Ok(())
    })?;

    Ok(summary)
}

/// Diário de todos os hábitos em CSV (separador ";").
pub fn export_diary_csv(db: &Db) -> rusqlite::Result<String> {
    let habits = db.habits()?;
    let mut rule_names = std::collections::HashMap::<i64, String>::new();
    for h in &habits {
        for r in db.rules(h.id)? {
            rule_names.insert(r.id, format!("{}: {}", r.kind.label(), r.text));
        }
    }
    let names: std::collections::HashMap<i64, String> = habits.into_iter().map(|h| (h.id, h.name)).collect();
    let mut out = String::from("habito;data;recaida;dinheiro;minutos;regra;nota\n");
    for e in db.all_entries()? {
        let date = chrono::DateTime::from_timestamp_millis(e.occurred_at)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let fields = [
            csv(names.get(&e.habit_id).map(String::as_str).unwrap_or("")),
            date,
            if e.is_reset { "sim".into() } else { "nao".into() },
            format!("{:.2}", e.money_spent),
            format!("{:.1}", e.time_spent_ms as f64 / 60_000.0),
            csv(e.rule_id.and_then(|id| rule_names.get(&id)).map(String::as_str).unwrap_or("")),
            csv(e.note.as_deref().unwrap_or("")),
        ];
        out.push_str(&fields.join(";"));
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------- Helpers

fn csv(v: &str) -> String {
    if v.contains(';') || v.contains('"') || v.contains('\n') {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn list<'a>(json: &'a Value, key: &str) -> Vec<&'a Value> {
    json.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|v| v.is_object()).collect())
        .unwrap_or_default()
}

fn sorted_by_position(mut items: Vec<&Value>) -> Vec<&Value> {
    items.sort_by_key(|v| int(v, "listPosition").unwrap_or(0));
    items
}

fn int(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn non_neg(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    };
    if n.is_finite() && n > 0.0 {
        n
    } else {
        0.0
    }
}

fn boolean(v: Option<&Value>, fallback: bool) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => s == "true" || s == "1",
        _ => fallback,
    }
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn coded<T>(v: Option<&Value>, from_code: fn(i64) -> T, from_legacy: fn(&str) -> T) -> T {
    match v {
        Some(Value::Number(n)) => from_code(n.as_i64().unwrap_or(-1)),
        Some(Value::String(s)) => from_legacy(s),
        _ => from_legacy(""),
    }
}

#[allow(dead_code)]
fn object(v: &Value) -> Option<&Map<String, Value>> {
    v.as_object()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{goal_presets, DEFAULT_AFFIRMATIONS};
    use crate::units::{MS_PER_DAY, MS_PER_MINUTE};

    const BASE: i64 = 1_735_689_600_000;

    #[test]
    fn exporta_e_reimporta_sem_perdas() {
        let db = Db::open_in_memory().unwrap();
        let mut h = Habit::new("Cigarro", BASE);
        h.icon = 2002;
        h.color = 1001;
        h.daily_money = 12.5;
        h.daily_minutes = 30.0;
        h.counter_style = CounterStyle::WeeksDays;
        let id = db.insert_habit(&h, true).unwrap();
        db.insert_entry(&DiaryEntry {
            money_spent: 3.0,
            note: Some("festa".into()),
            ..DiaryEntry::reset(id, BASE + MS_PER_DAY)
        })
        .unwrap();
        let gid = db.insert_goal(&Goal::personal(id, "Viagem", BASE + 90 * MS_PER_DAY, "#123456")).unwrap();
        db.set_forced_goal(id, Some(gid)).unwrap();
        db.insert_reward(&Reward { purchased_at: Some(BASE + 2 * MS_PER_DAY), ..Reward::new(id, "Tênis", 300.0) })
            .unwrap();
        db.insert_investment(&TimeInvestment {
            invested_ms: 90 * MS_PER_MINUTE,
            ..TimeInvestment::new(id, "Leitura", 30 * MS_PER_MINUTE)
        })
        .unwrap();
        db.insert_text_item(TextTable::Reason, id, "Saúde").unwrap();
        db.set_setting("currency", "EUR").unwrap();
        let allowed = db.insert_rule(id, RuleKind::Allowed, "Adesivo de nicotina").unwrap();
        db.insert_rule(id, RuleKind::Forbidden, "Cigarro").unwrap();
        db.insert_entry(&DiaryEntry {
            is_reset: false,
            rule_id: Some(allowed),
            ..DiaryEntry::reset(id, BASE + 3 * MS_PER_DAY)
        })
        .unwrap();

        let text = serde_json::to_string(&export_backup(&db, "teste").unwrap()).unwrap();
        let json: Value = serde_json::from_str(&text).unwrap();
        let other = Db::open_in_memory().unwrap();
        let summary = import_backup(&other, &json, true).unwrap();

        assert!(!summary.from_quitzilla);
        assert_eq!(summary.habits, 1);
        let h2 = other.habits().unwrap().remove(0);
        assert_eq!(h2.name, "Cigarro");
        assert_eq!(h2.icon, 2002);
        assert_eq!(h2.color, 1001);
        assert_eq!(h2.daily_money, 12.5);
        assert_eq!(h2.counter_style, CounterStyle::WeeksDays);
        let entries2 = other.entries(h2.id).unwrap();
        assert_eq!(entries2.len(), 2);
        assert!(entries2.iter().any(|e| e.note.as_deref() == Some("festa")));

        assert_eq!(summary.rules, 2);
        let rules2 = other.rules(h2.id).unwrap();
        assert_eq!(rules2.len(), 2);
        let allowed2 = rules2.iter().find(|r| r.kind == RuleKind::Allowed).unwrap();
        assert_eq!(allowed2.text, "Adesivo de nicotina");
        let linked = entries2.iter().find(|e| e.rule_id.is_some()).unwrap();
        assert_eq!(linked.rule_id, Some(allowed2.id));
        assert!(!linked.is_reset);

        let goals = other.goals(h2.id).unwrap();
        assert_eq!(goals.len(), goal_presets().len() + 1);
        let personal = goals.iter().find(|g| g.is_personal).unwrap();
        assert_eq!(personal.target_at, Some(BASE + 90 * MS_PER_DAY));
        assert!(personal.is_forced_current);
        assert_eq!(personal.hex_color, "123456");
        assert!(goals.iter().filter(|g| !g.is_personal).all(|g| g.preset_key.is_some()));

        assert_eq!(other.rewards(h2.id).unwrap()[0].purchased_at, Some(BASE + 2 * MS_PER_DAY));
        assert_eq!(other.investments(h2.id).unwrap()[0].invested_ms, 90 * MS_PER_MINUTE);
        assert_eq!(other.text_items(TextTable::Reason, h2.id).unwrap()[0].text, "Saúde");
        assert_eq!(other.text_items(TextTable::Affirmation, h2.id).unwrap().len(), DEFAULT_AFFIRMATIONS.len());
        assert_eq!(other.setting("currency").unwrap().as_deref(), Some("EUR"));
    }

    #[test]
    fn importa_backup_do_quitzilla() {
        let json = json!({
            "version": 3, "appVersion": "3.0.2", "createdAt": BASE,
            "habits": [{
                "id": 7, "name": "Smoking", "icon": "SMOKING", "color": "RED", "quitDate": BASE,
                "dailyMoneySpending": 20.0, "dailyTimeSpending": 45.0, "dailyFrequency": 10.0,
                "listPosition": 0, "preferredSpendingType": "MONEY",
                "counterType": "REMAINING", "counterStyle": "DAYS_HOURS"
            }],
            "diaryEntries": [{
                "id": 1, "habitId": 7, "timestamp": BASE + 3 * MS_PER_DAY, "moneySpent": 8.0,
                "timeSpent": 600000, "isReset": true, "comment": null
            }],
            "goals": [
                { "id": 1, "habitId": 7, "name": "MONTH_2", "durationMs": 5_260_000_000_i64,
                  "hexColor": "#0080E0", "isPersonal": false, "isForceCurrent": true },
                { "id": 2, "habitId": 7, "name": "Birthday", "durationMs": BASE + 200 * MS_PER_DAY,
                  "hexColor": "#FF0000", "isPersonal": true, "isForceCurrent": true }
            ],
            "trophies": [],
            "rewards": [{ "id": 1, "habitId": 7, "name": "Fone", "price": 150.0, "isPurchased": false,
                          "isDeleted": false, "listPosition": 0, "purchaseDate": null }],
            "timeInvestments": [{ "id": 1, "habitId": 7, "name": "Corrida", "sessionDuration": 1_800_000,
                                  "totalInvestments": 3_600_000, "isDeleted": false, "listPosition": 0 }],
            "reasons": [{ "id": 1, "habitId": 7, "text": "Respirar melhor", "listPosition": 0 }],
            "affirmations": [],
            "settings": { "currencyCode": "BRL", "mainTitle": "Um dia de cada vez" }
        });

        let db = Db::open_in_memory().unwrap();
        db.insert_habit(&Habit::new("Será substituído", BASE), true).unwrap();
        let summary = import_backup(&db, &json, true).unwrap();

        assert!(summary.from_quitzilla);
        let habits = db.habits().unwrap();
        assert_eq!(habits.len(), 1);
        let h = &habits[0];
        assert_eq!(h.name, "Smoking");
        assert_eq!(h.icon, 2002);
        assert_eq!(h.counter_type, CounterType::Remaining);
        assert_eq!(h.counter_style, CounterStyle::DaysHours);
        assert_eq!(h.daily_minutes, 45.0);

        let entry = &db.entries(h.id).unwrap()[0];
        assert!(entry.is_reset);
        assert_eq!(entry.time_spent_ms, 600_000);

        let goals = db.goals(h.id).unwrap();
        assert_eq!(goals.len(), 2);
        let preset = goals.iter().find(|g| !g.is_personal).unwrap();
        assert_eq!(preset.preset_key.as_deref(), Some("MONTH_2"));
        assert_eq!(preset.name, "2 meses");
        assert!(preset.is_forced_current);
        let personal = goals.iter().find(|g| g.is_personal).unwrap();
        assert_eq!(personal.target_at, Some(BASE + 200 * MS_PER_DAY));
        assert!(!personal.is_forced_current, "só a primeira meta fixada é mantida");

        assert_eq!(db.investments(h.id).unwrap()[0].invested_ms, 3_600_000);
        assert!(!db.rewards(h.id).unwrap()[0].is_purchased());
        assert_eq!(db.setting("motto").unwrap().as_deref(), Some("Um dia de cada vez"));
    }

    #[test]
    fn habito_de_check_diario_no_backup() {
        let db = Db::open_in_memory().unwrap();
        let mut h = Habit::new("Acordar cedo", BASE);
        h.kind = HabitKind::Build;
        h.icon = 3001;
        h.schedule_days = crate::checkin::WEEKDAYS;
        let id = db.insert_habit(&h, true).unwrap();
        db.set_checkin(id, "2025-01-06", true).unwrap();
        db.set_checkin(id, "2025-01-07", true).unwrap();
        let mut study = Habit::new("Estudar", BASE);
        study.kind = HabitKind::Build;
        study.daily_target = 50.0;
        study.target_unit = "min".into();
        let sid = db.insert_habit(&study, true).unwrap();
        db.set_progress(sid, "2025-01-06", 35.0).unwrap();
        db.insert_focus(&crate::pomodoro::FocusRecord {
            id: 0,
            habit_id: Some(sid),
            started_at: BASE,
            ended_at: BASE + 1_500_000,
            duration_ms: 1_500_000,
            mode: crate::pomodoro::Mode::Pomodoro,
        })
        .unwrap();

        let json = export_backup(&db, "teste").unwrap();
        let other = Db::open_in_memory().unwrap();
        let summary = import_backup(&other, &json, true).unwrap();
        assert_eq!(summary.checkins, 3);
        assert_eq!(summary.focus_sessions, 1);
        let mut habits2 = other.habits().unwrap();
        let s2 = habits2.remove(1);
        assert_eq!(s2.daily_target, 50.0);
        assert_eq!(s2.target_unit, "min");
        assert_eq!(other.checkins(s2.id).unwrap()[0].amount, 35.0);
        assert_eq!(other.focus_sessions_since(0).unwrap()[0].habit_id, Some(s2.id));
        let h2 = habits2.remove(0);
        assert_eq!(h2.kind, HabitKind::Build);
        assert_eq!(h2.icon, 3001);
        assert_eq!(h2.schedule_days, crate::checkin::WEEKDAYS);
        let days: Vec<String> = other.checkins(h2.id).unwrap().into_iter().map(|p| p.day).collect();
        assert_eq!(days, vec!["2025-01-06".to_string(), "2025-01-07".to_string()]);
        assert!(other.goals(h2.id).unwrap().is_empty());
    }

    #[test]
    fn arquivo_sem_habitos_e_rejeitado_sem_apagar_nada() {
        let db = Db::open_in_memory().unwrap();
        db.insert_habit(&Habit::new("Café", BASE), false).unwrap();
        let err = import_backup(&db, &json!({ "habits": [] }), true).unwrap_err();
        assert!(matches!(err, BackupError::Format(_)));
        assert_eq!(db.habits().unwrap().len(), 1);
    }

    #[test]
    fn csv_do_diario() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_habit(&Habit::new("Café", BASE), false).unwrap();
        db.insert_entry(&DiaryEntry {
            money_spent: 4.5,
            note: Some("com \"aspas\"; e ponto e vírgula".into()),
            ..DiaryEntry::reset(id, BASE + MS_PER_DAY)
        })
        .unwrap();
        let out = export_diary_csv(&db).unwrap();
        let lines: Vec<&str> = out.trim_end().lines().collect();
        assert_eq!(lines[0], "habito;data;recaida;dinheiro;minutos;regra;nota");
        assert!(lines[1].starts_with("Café;"));
        assert!(lines[1].ends_with("\"com \"\"aspas\"\"; e ponto e vírgula\""));
        assert!(lines[1].contains(";sim;4.50;0.0;;"));

        let rule = db.insert_rule(id, RuleKind::Allowed, "Café descafeinado").unwrap();
        db.insert_entry(&DiaryEntry { is_reset: false, rule_id: Some(rule), ..DiaryEntry::reset(id, BASE + 2 * MS_PER_DAY) })
            .unwrap();
        let out = export_diary_csv(&db).unwrap();
        assert!(out.contains(";nao;0.00;0.0;Permitido: Café descafeinado;"));
    }
}
