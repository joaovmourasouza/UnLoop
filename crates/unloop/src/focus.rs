//! Foco: timer (Pomodoro/cronômetro) e estatísticas.
//! O timer mora no contexto (`Ctx::timer`), então continua rodando fora desta tela.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use unloop_core::checkin::{self, local_date, NaiveDate};
use unloop_core::format::format_compact;
use unloop_core::model::Habit;
use unloop_core::pomodoro::{self, Credit, Mode, Period};
use unloop_core::units::MS_PER_MINUTE;

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};

struct StatsState {
    period: Cell<Period>,
    anchor: Cell<NaiveDate>,
}

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let stack = adw::ViewStack::new();
    let timer_box = widgets::content_box();
    let stats_box = widgets::content_box();
    stack
        .add_titled(&widgets::scroller(&timer_box), Some("timer"), "Timer")
        .set_icon_name(Some("alarm-symbolic"));
    stack
        .add_titled(&widgets::scroller(&stats_box), Some("stats"), "Estatísticas")
        .set_icon_name(Some("view-list-symbolic"));

    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    let page = adw::NavigationPage::builder().title("Foco").child(&toolbar).build();

    let state = Rc::new(StatsState { period: Cell::new(Period::Week), anchor: Cell::new(checkin::today()) });
    timer_tab(ctx, &timer_box);
    stats_tab(ctx, &stats_box, &state);

    let (c, wt, ws) = (ctx.clone(), timer_box.downgrade(), stats_box.downgrade());
    ctx.on_refresh(move || match (wt.upgrade(), ws.upgrade()) {
        (Some(t), Some(s)) => {
            widgets::clear_box(&t);
            widgets::clear_box(&s);
            timer_tab(&c, &t);
            stats_tab(&c, &s, &state);
            true
        }
        _ => false,
    });
    page
}

fn credit_hint(h: &Habit) -> String {
    match pomodoro::focus_credit(h, 25 * MS_PER_MINUTE) {
        Credit::AddProgress(_) => format!("Os minutos de foco somam na meta diária ({}).", h.target_unit),
        Credit::Check => "Uma sessão de foco marca o dia como feito.".to_string(),
        Credit::None => format!("A meta deste hábito é em \"{}\": o foco fica só no histórico.", h.target_unit),
    }
}

// ------------------------------------------------------------------- Timer

fn timer_tab(ctx: &Shared, b: &gtk::Box) {
    let timer = ctx.timer.borrow().clone();
    let settings = ctx.pomodoro_settings.borrow().clone();

    // Modo
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    modes.set_halign(gtk::Align::Center);
    for (mode, label) in [(Mode::Pomodoro, "Pomodoro"), (Mode::Stopwatch, "Cronômetro")] {
        let button = gtk::Button::with_label(label);
        if timer.mode == mode {
            button.add_css_class("suggested-action");
        }
        let c = ctx.clone();
        button.connect_clicked(move |_| {
            if c.timer.borrow().mode == mode {
                return;
            }
            if !c.timer.borrow().is_idle() {
                c.toast("Encerre a sessão atual para trocar de modo.");
                return;
            }
            c.timer.borrow_mut().set_mode(mode);
            changed_later(&c);
        });
        modes.append(&button);
    }
    b.append(&modes);

    let phase = widgets::label("", &["title-3"]);
    let time = widgets::label("", &["timer"]);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("thick");
    progress.set_visible(timer.mode == Mode::Pomodoro);
    let cycle = widgets::label("", &["dim-label"]);
    b.append(&phase);
    b.append(&time);
    b.append(&progress);
    b.append(&cycle);

    // Controles
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_halign(gtk::Align::Center);
    let primary = widgets::pill_button("Iniciar", Some("suggested-action"));
    let stop = widgets::pill_button("Encerrar", None);
    let skip = widgets::pill_button("Pular", None);
    skip.set_visible(timer.mode == Mode::Pomodoro);
    controls.append(&primary);
    controls.append(&stop);
    controls.append(&skip);
    b.append(&controls);

    let c = ctx.clone();
    primary.connect_clicked(move |_| {
        let now = unloop_core::now_ms();
        {
            let mut t = c.timer.borrow_mut();
            if t.is_running() {
                t.pause(now);
            } else {
                t.start(now);
            }
        }
        changed_later(&c);
    });
    let c = ctx.clone();
    stop.connect_clicked(move |_| {
        let now = unloop_core::now_ms();
        let was_idle = c.timer.borrow().is_idle();
        let record = c.timer.borrow_mut().stop(now);
        match record {
            Some(r) => {
                c.save_focus(&r);
                c.toast(&format!("Sessão registrada: {}", format_compact(r.duration_ms)));
            }
            None if !was_idle => c.toast("Sessões com menos de 1 minuto não são registradas."),
            None => {}
        }
        changed_later(&c);
    });
    let c = ctx.clone();
    skip.connect_clicked(move |_| {
        let now = unloop_core::now_ms();
        let settings = c.pomodoro_settings.borrow().clone();
        let record = c.timer.borrow_mut().skip(now, &settings);
        if let Some(r) = record {
            c.save_focus(&r);
        }
        changed_later(&c);
    });

    // Atualização a cada segundo.
    let c = ctx.clone();
    let weak = (phase.downgrade(), time.downgrade(), progress.downgrade(), cycle.downgrade(), primary.downgrade());
    ctx.live(move |now| {
        let (Some(phase), Some(time), Some(progress), Some(cycle), Some(primary)) =
            (weak.0.upgrade(), weak.1.upgrade(), weak.2.upgrade(), weak.3.upgrade(), weak.4.upgrade())
        else {
            return false;
        };
        let t = c.timer.borrow();
        let s = c.pomodoro_settings.borrow();
        phase.set_label(if t.mode == Mode::Stopwatch { "Cronômetro" } else { t.phase.label() });
        time.set_label(&widgets::clock(t.display_ms(now, &s)));
        progress.set_fraction(t.progress(now, &s));
        let every = s.long_break_every.max(1);
        cycle.set_label(&if t.mode == Mode::Pomodoro && t.phase.is_focus() {
            format!("Foco {} de {} até a pausa longa", t.focus_count % every + 1, every)
        } else {
            String::new()
        });
        primary.set_label(if t.is_running() {
            "Pausar"
        } else if t.is_idle() {
            "Iniciar"
        } else {
            "Retomar"
        });
        true
    });

    // Hábito vinculado
    let habits: Vec<Habit> =
        ctx.check(ctx.db.habits()).unwrap_or_default().into_iter().filter(|h| h.is_build()).collect();
    let link = widgets::group("Hábito vinculado", Some("O foco concluído é creditado neste hábito."));
    let mut labels = vec!["Nenhum".to_string()];
    labels.extend(habits.iter().map(|h| format!("{}  {}", h.icon_info().emoji, h.name)));
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let combo = adw::ComboRow::builder()
        .title("Hábito")
        .model(&gtk::StringList::new(&label_refs))
        .build();
    let selected = timer.habit_id.and_then(|id| habits.iter().position(|h| h.id == id)).map_or(0, |i| i + 1);
    combo.set_selected(selected as u32);
    combo.set_subtitle(&if selected == 0 {
        "Crie um hábito de check diário (ex.: Estudar 50 min) para acumular o foco nele.".to_string()
    } else {
        credit_hint(&habits[selected - 1])
    });
    let c = ctx.clone();
    combo.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        let habit = index.checked_sub(1).and_then(|i| habits.get(i));
        c.timer.borrow_mut().habit_id = habit.map(|h| h.id);
        row.set_subtitle(&habit.map_or_else(|| "Sem vínculo: o foco fica só no histórico.".to_string(), credit_hint));
    });
    link.add(&combo);

    // Tarefa em que se está trabalhando (aparece nas estatísticas e na CLI).
    let open_tasks: Vec<unloop_core::model::Task> =
        ctx.check(ctx.db.tasks()).unwrap_or_default().into_iter().filter(|t| !t.is_done()).collect();
    let mut task_labels = vec!["Nenhuma".to_string()];
    task_labels.extend(open_tasks.iter().map(|t| t.title.clone()));
    let task_refs: Vec<&str> = task_labels.iter().map(String::as_str).collect();
    let task_combo = adw::ComboRow::builder()
        .title("Tarefa")
        .subtitle("Em que você vai trabalhar neste foco")
        .model(&gtk::StringList::new(&task_refs))
        .build();
    let selected_task = timer.task_id.and_then(|id| open_tasks.iter().position(|t| t.id == id)).map_or(0, |i| i + 1);
    task_combo.set_selected(selected_task as u32);
    let c = ctx.clone();
    task_combo.connect_selected_notify(move |row| {
        let task = (row.selected() as usize).checked_sub(1).and_then(|i| open_tasks.get(i));
        c.timer.borrow_mut().task_id = task.map(|t| t.id);
    });
    link.add(&task_combo);
    b.append(&link);

    // Configurações
    let config = widgets::group("Configurações do Pomodoro", None);
    let spin = |title: &str, value: u32, max: f64| {
        let row = adw::SpinRow::with_range(1.0, max, 1.0);
        row.set_title(title);
        row.set_value(f64::from(value));
        row
    };
    let focus_row = spin("Foco (min)", settings.focus_min, 240.0);
    let short_row = spin("Pausa curta (min)", settings.short_break_min, 60.0);
    let long_row = spin("Pausa longa (min)", settings.long_break_min, 120.0);
    let every_row = spin("Pausa longa a cada (focos)", settings.long_break_every, 12.0);
    let auto_breaks = adw::SwitchRow::builder().title("Iniciar pausas automaticamente").active(settings.auto_start_breaks).build();
    let auto_focus = adw::SwitchRow::builder().title("Iniciar o próximo foco automaticamente").active(settings.auto_start_focus).build();
    for row in [&focus_row, &short_row, &long_row, &every_row] {
        config.add(row);
    }
    config.add(&auto_breaks);
    config.add(&auto_focus);
    b.append(&config);

    let save = {
        let c = ctx.clone();
        let rows = (focus_row.clone(), short_row.clone(), long_row.clone(), every_row.clone());
        let switches = (auto_breaks.clone(), auto_focus.clone());
        move || {
            let s = pomodoro::Settings {
                focus_min: rows.0.value() as u32,
                short_break_min: rows.1.value() as u32,
                long_break_min: rows.2.value() as u32,
                long_break_every: rows.3.value() as u32,
                auto_start_breaks: switches.0.is_active(),
                auto_start_focus: switches.1.is_active(),
            };
            if c.check(s.save(&c.db)).is_some() {
                *c.pomodoro_settings.borrow_mut() = s;
            }
        }
    };
    let save = Rc::new(save);
    for row in [&focus_row, &short_row, &long_row, &every_row] {
        let s = save.clone();
        row.connect_value_notify(move |_| (*s)());
    }
    for row in [&auto_breaks, &auto_focus] {
        let s = save.clone();
        row.connect_active_notify(move |_| (*s)());
    }
}

// ----------------------------------------------------------- Estatísticas

fn rerender_stats(ctx: &Shared, b: &gtk::Box, state: &Rc<StatsState>) {
    let (c, wb, st) = (ctx.clone(), b.downgrade(), state.clone());
    glib::idle_add_local_once(move || {
        if let Some(b) = wb.upgrade() {
            widgets::clear_box(&b);
            stats_tab(&c, &b, &st);
        }
    });
}

fn stats_tab(ctx: &Shared, b: &gtk::Box, state: &Rc<StatsState>) {
    let records = ctx.check(ctx.db.focus_sessions_since(0)).unwrap_or_default();
    let all_habits = ctx.check(ctx.db.habits()).unwrap_or_default();
    let names: HashMap<i64, String> =
        all_habits.iter().map(|h| (h.id, format!("{} {}", h.icon_info().emoji, h.name))).collect();
    let colors: HashMap<i64, String> = all_habits.iter().map(|h| (h.id, h.color_info().hex.to_string())).collect();
    let habit_name = |id: Option<i64>| id.and_then(|i| names.get(&i).cloned()).unwrap_or_else(|| "Sem hábito".to_string());
    let today = checkin::today();

    // Visão geral
    let today_totals = pomodoro::totals(records.iter().filter(|r| local_date(r.started_at) == today));
    let all = pomodoro::totals(&records);
    let overview = widgets::group("Visão geral", None);
    for (title, value) in [
        ("🍅 Pomodoros hoje", today_totals.pomodoros.to_string()),
        ("Foco hoje", format_compact(today_totals.ms)),
        ("🍅 Pomodoros no total", all.pomodoros.to_string()),
        ("Foco no total", format_compact(all.ms)),
        ("Sessões no total", all.sessions.to_string()),
        ("Dias seguidos com foco", pomodoro::focus_day_streak(&records, today).to_string()),
    ] {
        overview.add(&widgets::value_row(title, &value).0);
    }
    b.append(&overview);

    if records.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("alarm-symbolic")
            .title("Nenhuma sessão ainda")
            .description("Inicie um Pomodoro na aba Timer. As estatísticas aparecem aqui.")
            .build();
        b.append(&status);
        return;
    }

    // Período
    let period = state.period.get();
    let anchor = state.anchor.get();
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let periods = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    periods.add_css_class("linked");
    for (p, label) in [(Period::Week, "Semana"), (Period::Month, "Mês"), (Period::Year, "Ano")] {
        let button = gtk::Button::with_label(label);
        if p == period {
            button.add_css_class("suggested-action");
        }
        let (c, wb, st) = (ctx.clone(), b.downgrade(), state.clone());
        button.connect_clicked(move |_| {
            st.period.set(p);
            if let Some(b) = wb.upgrade() {
                rerender_stats(&c, &b, &st);
            }
        });
        periods.append(&button);
    }
    controls.append(&periods);
    let title = gtk::Label::new(Some(&pomodoro::period_label(period, anchor)));
    title.add_css_class("heading");
    title.set_hexpand(true);
    let prev = widgets::icon_button("go-previous-symbolic", "Período anterior");
    let next = widgets::icon_button("go-next-symbolic", "Próximo período");
    next.set_sensitive(pomodoro::period_range(period, anchor).1 < today);
    for (button, delta) in [(&prev, -1), (&next, 1)] {
        let (c, wb, st) = (ctx.clone(), b.downgrade(), state.clone());
        button.connect_clicked(move |_| {
            st.anchor.set(pomodoro::shift_period(st.period.get(), st.anchor.get(), delta));
            if let Some(b) = wb.upgrade() {
                rerender_stats(&c, &b, &st);
            }
        });
    }
    controls.append(&prev);
    controls.append(&title);
    controls.append(&next);
    b.append(&controls);

    // Gráfico do período
    let series = pomodoro::series(&records, period, anchor);
    let selected = pomodoro::in_period(&records, period, anchor);
    let period_totals = pomodoro::totals(&selected);
    let (start, end) = pomodoro::period_range(period, anchor);
    let elapsed_days = ((end.min(today) - start).num_days() + 1).max(1);
    let chart_summary = format!(
        "Total: {} · {} 🍅 · média diária: {}",
        format_compact(period_totals.ms),
        period_totals.pomodoros,
        format_compact(period_totals.ms / elapsed_days)
    );
    let chart_group = widgets::group("Tempo de foco", Some(chart_summary.as_str()));
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    let chart = widgets::bar_chart(&series, None);
    chart.set_margin_top(16);
    chart.set_margin_bottom(12);
    chart.set_margin_start(12);
    chart.set_margin_end(12);
    card.append(&chart);
    chart_group.add(&card);

    // Comparação com o período anterior
    let comparison = pomodoro::compare_with_previous(&records, period, anchor, today);
    let trend = match comparison.time_change() {
        Some(c) if c > 0.0 => "📈",
        Some(c) if c < 0.0 => "📉",
        _ => "➖",
    };
    let pomodoro_trend = comparison
        .pomodoro_change()
        .map(|c| format!(" ({}{:.0}%)", if c >= 0.0 { "+" } else { "−" }, (c * 100.0).abs()))
        .unwrap_or_default();
    let comparison_row = adw::ActionRow::builder()
        .title(esc(&pomodoro::comparison_text(&comparison, period)))
        .subtitle(esc(&format!(
            "Agora: {} · {} 🍅{} — antes: {} · {} 🍅",
            format_compact(comparison.current.ms),
            comparison.current.pomodoros,
            pomodoro_trend,
            format_compact(comparison.previous.ms),
            comparison.previous.pomodoros
        )))
        .build();
    comparison_row.add_prefix(&gtk::Label::new(Some(trend)));
    chart_group.add(&comparison_row);
    b.append(&chart_group);

    if selected.is_empty() {
        b.append(&widgets::label("Nenhuma sessão neste período.", &["dim-label"]));
        return;
    }

    // Distribuição por hábito
    // Cor de cada fatia: a cor do hábito; se repetir, uma cor da paleta.
    const PALETTE: [&str; 8] = ["3584E4", "2EC27E", "F5C211", "E66100", "C061CB", "865E3C", "1C71D8", "E01B24"];
    let mut used = std::collections::HashSet::new();
    let slices: Vec<(Option<i64>, i64, String)> = pomodoro::by_habit(&selected)
        .into_iter()
        .enumerate()
        .map(|(i, (habit_id, ms))| {
            let mut hex = habit_id.and_then(|id| colors.get(&id).cloned()).unwrap_or_else(|| "9A9996".to_string());
            if !used.insert(hex.clone()) {
                hex = PALETTE[i % PALETTE.len()].to_string();
                used.insert(hex.clone());
            }
            (habit_id, ms, hex)
        })
        .collect();

    let distribution = widgets::group("Distribuição por hábito", None);
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    let donut = widgets::donut_chart(
        slices.iter().map(|(_, ms, hex)| (*ms as f64, hex.clone())).collect(),
        &format!("{}\nno período", format_compact(period_totals.ms)),
        180,
    );
    layout.append(&donut);
    let legend = widgets::boxed_list();
    legend.set_hexpand(true);
    legend.set_valign(gtk::Align::Center);
    for (habit_id, ms, hex) in &slices {
        let share = *ms as f64 / period_totals.ms.max(1) as f64;
        let row = adw::ActionRow::builder()
            .title(esc(&habit_name(*habit_id)))
            .subtitle(format!("{} · {:.0}%", format_compact(*ms), share * 100.0))
            .build();
        let dot = gtk::Label::new(None);
        dot.set_markup(&format!("<span foreground=\"#{hex}\" size=\"x-large\">●</span>"));
        row.add_prefix(&dot);
        legend.append(&row);
    }
    layout.append(&legend);
    distribution.add(&layout);
    b.append(&distribution);

    // Melhor horário
    let hours = pomodoro::by_hour(&selected);
    let best = hours.iter().enumerate().max_by_key(|(_, ms)| **ms).map(|(h, _)| h).unwrap_or(0);
    let hour_series: Vec<(String, i64)> = hours.iter().enumerate().map(|(h, ms)| (format!("{h}h"), *ms)).collect();
    let best_text = format!("Você mais começa sessões de foco entre {best}h e {}h.", (best + 1) % 24);
    let hours_group = widgets::group("Melhor horário", Some(best_text.as_str()));
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    let chart = widgets::bar_chart(&hour_series, None);
    chart.set_margin_top(16);
    chart.set_margin_bottom(12);
    chart.set_margin_start(12);
    chart.set_margin_end(12);
    card.append(&chart);
    hours_group.add(&card);
    b.append(&hours_group);

    // Registros do período, por dia (mais recentes primeiro)
    let mut by_day: Vec<(NaiveDate, Vec<&pomodoro::FocusRecord>)> = Vec::new();
    for r in &selected {
        let day = local_date(r.started_at);
        match by_day.iter_mut().find(|(d, _)| *d == day) {
            Some(slot) => slot.1.push(r),
            None => by_day.push((day, vec![r])),
        }
    }
    by_day.sort_by(|a, b| b.0.cmp(&a.0));
    for (day, items) in by_day.into_iter().take(14) {
        let total: i64 = items.iter().map(|r| r.duration_ms).sum();
        let group = widgets::group(
            &format!("{} · {}", day.format("%d/%m/%Y"), format_compact(total)),
            None,
        );
        let list = widgets::boxed_list();
        for r in items {
            let mode = if r.mode == Mode::Pomodoro { "🍅" } else { "⏱️" };
            let row = adw::ActionRow::builder()
                .title(esc(&format!("{mode} {} · {}", format_compact(r.duration_ms), habit_name(r.habit_id))))
                .subtitle(format!("{} – {}", widgets::fmt_time(r.started_at), widgets::fmt_time(r.ended_at)))
                .build();
            let delete = widgets::icon_button("user-trash-symbolic", "Apagar sessão");
            let (c, id) = (ctx.clone(), r.id);
            delete.connect_clicked(move |_| {
                let c2 = c.clone();
                widgets::confirm(
                    &c,
                    "Apagar sessão?",
                    "A sessão sai do histórico. O progresso já creditado no hábito não é desfeito.",
                    "Apagar",
                    true,
                    move || {
                        if c2.check(c2.db.delete_focus(id)).is_some() {
                            c2.changed();
                        }
                    },
                );
            });
            row.add_suffix(&delete);
            list.append(&row);
        }
        group.add(&list);
        b.append(&group);
    }
}
