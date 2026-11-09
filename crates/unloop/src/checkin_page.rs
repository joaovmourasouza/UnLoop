//! Detalhe de um hábito de check diário ("criar um hábito").

use std::rc::Rc;

use adw::prelude::*;
use unloop_core::calc::HabitBundle;
use unloop_core::checkin::{
    self, day_key, describe_mask, is_scheduled, monday_of, trophy_days, week_days, CheckinStats, Datelike, NaiveDate,
    WEEKDAY_LETTER, WEEKDAY_SHORT,
};
use unloop_core::format::{format_compact, format_number};
use unloop_core::model::{color, Habit, Trophy};
use unloop_core::units::MS_PER_DAY;

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};
use crate::{detail, habit_form};

struct Tabs {
    overview: gtk::Box,
    history: gtk::Box,
    notes: gtk::Box,
    stats: gtk::Box,
    trophies: gtk::Box,
    motivation: gtk::Box,
}

pub fn page(ctx: &Shared, habit_id: i64) -> adw::NavigationPage {
    let stack = adw::ViewStack::new();
    let make = |name: &str, title: &str, icon: &str| -> gtk::Box {
        let content = widgets::content_box();
        let page = stack.add_titled(&widgets::scroller(&content), Some(name), title);
        page.set_icon_name(Some(icon));
        content
    };
    let tabs = Rc::new(Tabs {
        overview: make("today", "Hoje", "object-select-symbolic"),
        history: make("history", "Histórico", "x-office-calendar-symbolic"),
        notes: make("notes", "Notas", "accessories-text-editor-symbolic"),
        stats: make("stats", "Estatísticas", "view-list-symbolic"),
        trophies: make("trophies", "Troféus", "starred-symbolic"),
        motivation: make("motivation", "Motivação", "weather-clear-symbolic"),
    });

    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let edit = widgets::icon_button("document-edit-symbolic", "Editar hábito");
    header.pack_end(&detail::build_more_menu(ctx, habit_id));
    header.pack_end(&edit);

    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_bottom_bar(&switcher_bar);
    toolbar.set_content(Some(&stack));
    let bin = adw::BreakpointBin::builder().width_request(360).height_request(300).child(&toolbar).build();
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 700sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&switcher, "visible", Some(&false.to_value()));
        breakpoint.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
        bin.add_breakpoint(breakpoint);
    }
    let page = adw::NavigationPage::builder().title("Hábito").child(&bin).build();

    let c = ctx.clone();
    edit.connect_clicked(move |_| {
        if let Some(h) = c.check(c.db.habit(habit_id)).flatten() {
            c.nav.push(&habit_form::page(&c, Some(h)));
        }
    });

    rebuild(ctx, habit_id, &tabs, &page);
    let (c, t, wp) = (ctx.clone(), tabs.clone(), page.downgrade());
    ctx.on_refresh(move || match wp.upgrade() {
        Some(p) => rebuild(&c, habit_id, &t, &p),
        None => false,
    });
    page
}

fn rebuild(ctx: &Shared, habit_id: i64, tabs: &Tabs, page: &adw::NavigationPage) -> bool {
    let Some(bundle) = ctx.check(ctx.db.bundle(habit_id)).flatten() else { return false };
    page.set_title(&bundle.habit.name);
    let today = checkin::today();
    let stats = bundle.checkin_stats(today);
    for b in [&tabs.overview, &tabs.history, &tabs.notes, &tabs.stats, &tabs.trophies, &tabs.motivation] {
        widgets::clear_box(b);
    }
    overview(ctx, &bundle, &stats, today, &tabs.overview);
    history(ctx, &bundle, today, &tabs.history);
    crate::notes::tab(ctx, habit_id, &tabs.notes);
    stats_tab(ctx, &bundle, &stats, &tabs.stats);
    trophies(&stats, &tabs.trophies);
    detail::motivation(ctx, &bundle, &tabs.motivation);
    true
}

fn start_date(h: &Habit) -> NaiveDate {
    checkin::local_date(h.quit_at)
}

fn target_text(h: &Habit, amount: f64) -> String {
    format!("{} de {} {}", format_number(amount), format_number(h.daily_target), h.target_unit)
}

/// Registra progresso (com meta) ou alterna o check (sem meta) num dia.
fn edit_day(ctx: &Shared, h: &Habit, day: NaiveDate, current: f64) {
    let (id, key) = (h.id, day_key(day));
    if h.has_target() {
        let c = ctx.clone();
        widgets::ask_number(
            ctx,
            &format!("{} — {}", h.name, day.format("%d/%m/%Y")),
            &format!("Total do dia (meta: {} {}). Use 0 para apagar.", format_number(h.daily_target), h.target_unit),
            current,
            1,
            &h.target_unit,
            move |total| {
                if c.check(c.db.set_progress(id, &key, total)).is_some() {
                    c.changed();
                }
            },
        );
    } else if ctx.check(ctx.db.set_checkin(id, &key, current <= 0.0)).is_some() {
        changed_later(ctx);
    }
}

// ------------------------------------------------------------------- Hoje

fn overview(ctx: &Shared, bundle: &HabitBundle, stats: &CheckinStats, today: NaiveDate, b: &gtk::Box) {
    let h = &bundle.habit;
    let amount = bundle.amount_on(today);
    b.append(&widgets::avatar(h.icon, h.color, true));
    b.append(&widgets::label(&format!("🔥 {}", stats.current_streak), &["counter"]));
    b.append(&widgets::label(
        &format!("dias programados seguidos · melhor: {}", stats.best_streak),
        &["dim-label"],
    ));

    let today_group = widgets::group(
        "Hoje",
        Some(if stats.scheduled_today { "Dia programado." } else { "Hoje é folga: marcar conta como bônus e não quebra a sequência." }),
    );
    if h.has_target() {
        let row = adw::ActionRow::builder().title(esc(&target_text(h, amount))).build();
        let bar = gtk::ProgressBar::new();
        bar.set_fraction((amount / h.daily_target).clamp(0.0, 1.0));
        bar.set_valign(gtk::Align::Center);
        bar.set_width_request(160);
        bar.add_css_class("thick");
        bar.add_css_class(&format!("bar-color-{}", color(h.color).code));
        row.add_suffix(&bar);
        if stats.done_today {
            row.set_subtitle("Meta do dia batida ✓");
        } else {
            row.set_subtitle(&format!("Faltam {} {}", format_number((h.daily_target - amount).max(0.0)), h.target_unit));
        }
        today_group.add(&row);
    }
    b.append(&today_group);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    if h.has_target() {
        let add = widgets::pill_button("Registrar progresso", Some("suggested-action"));
        let (c, id, unit, key) = (ctx.clone(), h.id, h.target_unit.clone(), day_key(today));
        add.connect_clicked(move |_| {
            let (c2, key) = (c.clone(), key.clone());
            widgets::ask_number(&c, "Registrar progresso", "Quanto você fez agora? Será somado ao total de hoje.", 0.0, 1, &unit, move |delta| {
                if delta > 0.0 && c2.check(c2.db.add_progress(id, &key, delta)).is_some() {
                    c2.changed();
                }
            });
        });
        buttons.append(&add);
        let adjust = widgets::pill_button("Ajustar total", None);
        let (c, habit) = (ctx.clone(), h.clone());
        adjust.connect_clicked(move |_| edit_day(&c, &habit, today, amount));
        buttons.append(&adjust);
    } else {
        let toggle = if stats.done_today {
            widgets::pill_button("✓ Feito hoje (desfazer)", None)
        } else {
            widgets::pill_button("Marcar como feito hoje", Some("suggested-action"))
        };
        let (c, habit) = (ctx.clone(), h.clone());
        toggle.connect_clicked(move |_| edit_day(&c, &habit, today, amount));
        buttons.append(&toggle);
    }
    let focus_button = widgets::pill_button("🍅 Iniciar foco", None);
    focus_button.set_tooltip_text(Some("Abre o Pomodoro vinculado a este hábito"));
    let (c, id) = (ctx.clone(), h.id);
    focus_button.connect_clicked(move |_| {
        c.timer.borrow_mut().habit_id = Some(id);
        crate::shell::go(&c, crate::shell::Section::Focus);
    });
    buttons.append(&focus_button);
    b.append(&buttons);

    // Últimos 7 dias
    let week = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    week.set_halign(gtk::Align::Center);
    let start = start_date(h);
    for back in (0..7u64).rev() {
        let Some(day) = today.checked_sub_days(unloop_core::checkin::Days::new(back)) else { continue };
        let done = unloop_core::checkin::day_done(bundle.amount_on(day), h.daily_target);
        let scheduled = is_scheduled(h.schedule_days, day);
        let mark = if day < start {
            "·"
        } else if done {
            "✅"
        } else if !scheduled {
            "–"
        } else if day == today {
            "⏳"
        } else {
            "❌"
        };
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        cell.add_css_class("week-day");
        let letter = gtk::Label::new(Some(WEEKDAY_LETTER[day.weekday().num_days_from_monday() as usize]));
        letter.add_css_class("dim-label");
        let symbol = gtk::Label::new(Some(mark));
        symbol.add_css_class("mark");
        cell.append(&letter);
        cell.append(&symbol);
        cell.set_tooltip_text(Some(&day.format("%d/%m").to_string()));
        week.append(&cell);
    }
    b.append(&week);

    let info = widgets::group("Programação", None);
    info.add(&widgets::value_row("Dias", &describe_mask(h.schedule_days)).0);
    info.add(&widgets::value_row(
        "Meta diária",
        &if h.has_target() {
            format!("{} {}", format_number(h.daily_target), h.target_unit)
        } else {
            "Só marcar feito".to_string()
        },
    ).0);
    b.append(&info);

    if !bundle.affirmations.is_empty() {
        let index = (unloop_core::now_ms() / MS_PER_DAY) as usize % bundle.affirmations.len();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        let text = widgets::label(&format!("“{}”", bundle.affirmations[index].text), &["affirmation"]);
        text.set_margin_top(18);
        text.set_margin_bottom(18);
        text.set_margin_start(18);
        text.set_margin_end(18);
        card.append(&text);
        b.append(&card);
    }
}

// -------------------------------------------------------------- Histórico

fn history(ctx: &Shared, bundle: &HabitBundle, today: NaiveDate, b: &gtk::Box) {
    let h = &bundle.habit;
    let start = start_date(h);
    b.append(&widgets::label(
        if h.has_target() {
            "Clique num dia para ajustar o total. Dias esmaecidos não estão programados."
        } else {
            "Clique num dia para marcar ou desmarcar. Dias esmaecidos não estão programados."
        },
        &["dim-label"],
    ));

    let grid = gtk::Grid::builder().row_spacing(6).column_spacing(6).halign(gtk::Align::Center).build();
    for (col, name) in WEEKDAY_SHORT.iter().enumerate() {
        let l = gtk::Label::new(Some(name));
        l.add_css_class("caption");
        l.add_css_class("dim-label");
        grid.attach(&l, col as i32, 0, 1, 1);
    }
    let weeks = 12u64;
    let first_monday = monday_of(today)
        .checked_sub_days(unloop_core::checkin::Days::new(7 * (weeks - 1)))
        .unwrap_or(today);
    for w in 0..weeks {
        let Some(monday) = first_monday.checked_add_days(unloop_core::checkin::Days::new(7 * w)) else { continue };
        for (col, day) in week_days(monday).into_iter().enumerate() {
            let amount = bundle.amount_on(day);
            let done = unloop_core::checkin::day_done(amount, h.daily_target);
            let button = gtk::Button::with_label(&day.day().to_string());
            button.add_css_class("day-cell");
            if done {
                button.add_css_class("suggested-action");
            } else if amount > 0.0 {
                button.add_css_class("partial");
            }
            if !is_scheduled(h.schedule_days, day) {
                button.add_css_class("off");
            }
            if day == today {
                button.add_css_class("accent");
            }
            let tooltip = if h.has_target() {
                format!("{} — {}", day.format("%d/%m/%Y"), target_text(h, amount))
            } else {
                day.format("%d/%m/%Y").to_string()
            };
            button.set_tooltip_text(Some(&tooltip));
            if day < start || day > today {
                button.set_sensitive(false);
            } else {
                let (c, habit) = (ctx.clone(), h.clone());
                button.connect_clicked(move |_| edit_day(&c, &habit, day, amount));
            }
            grid.attach(&button, col as i32, (w + 1) as i32, 1, 1);
        }
    }
    b.append(&grid);
}

// ----------------------------------------------------------- Estatísticas

fn stats_tab(ctx: &Shared, bundle: &HabitBundle, stats: &CheckinStats, b: &gtk::Box) {
    let h = &bundle.habit;
    let group = widgets::group("Sequência", None);
    for (title, value) in [
        ("Sequência atual", format!("{} dias", stats.current_streak)),
        ("Melhor sequência", format!("{} dias", stats.best_streak)),
        ("Taxa de conclusão", format!("{:.0}%", stats.completion_rate() * 100.0)),
        (
            "Dias concluídos",
            format!("{} de {} programados", stats.completed_scheduled, stats.scheduled_days),
        ),
        ("Total de dias feitos (inclui bônus)", stats.total_checks.to_string()),
        ("Início", start_date(h).format("%d/%m/%Y").to_string()),
        ("Troféus", format!("{} de {}", stats.trophies_achieved(), Trophy::ALL.len())),
    ] {
        group.add(&widgets::value_row(title, &value).0);
    }
    b.append(&group);

    if h.has_target() {
        let total: f64 = bundle.checkins.iter().map(|p| p.amount).sum();
        let days = bundle.checkins.len().max(1) as f64;
        let progress = widgets::group("Progresso acumulado", None);
        progress.add(&widgets::value_row("Total", &format!("{} {}", format_number(total), h.target_unit)).0);
        progress.add(&widgets::value_row("Média por dia registrado", &format!("{} {}", format_number(total / days), h.target_unit)).0);
        b.append(&progress);
    }

    let focus_ms: i64 = ctx
        .check(ctx.db.focus_sessions_since(0))
        .unwrap_or_default()
        .iter()
        .filter(|f| f.habit_id == Some(h.id))
        .map(|f| f.duration_ms)
        .sum();
    if focus_ms > 0 {
        let focus = widgets::group("Foco", None);
        focus.add(&widgets::value_row("Tempo de foco (Pomodoro)", &format_compact(focus_ms)).0);
        b.append(&focus);
    }
}

// ----------------------------------------------------------------- Troféus

fn trophies(stats: &CheckinStats, b: &gtk::Box) {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(6)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    for &t in Trophy::ALL {
        let at = stats.trophy_at(t);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("card");
        card.add_css_class("trophy");
        if at.is_none() {
            card.add_css_class("locked");
        }
        let emoji = if t == Trophy::DecisionMade { "🤝" } else if at.is_some() { "🏆" } else { "🔒" };
        card.append(&widgets::label(emoji, &["emoji"]));
        let name = if t == Trophy::DecisionMade {
            "Primeiro passo".to_string()
        } else {
            format!("{} seguidos", trophy_days(t))
        };
        card.append(&widgets::label(&name, &["heading"]));
        let status = match at {
            Some(day) => format!("Conquistado em {}", day.format("%d/%m/%Y")),
            None => format!("Faltam {} dias", trophy_days(t).saturating_sub(stats.current_streak)),
        };
        card.append(&widgets::label(&status, &["caption", "dim-label"]));
        flow.append(&card);
    }
    b.append(&flow);
}
