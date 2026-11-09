//! Seções "Hoje" (resumo do dia: números, tarefas e hábitos de hoje) e
//! "Hábitos" (todos os hábitos, contadores ao vivo e totais).

use adw::prelude::*;
use unloop_core::calc::HabitBundle;
use unloop_core::checkin::{self, day_key};
use unloop_core::format::{format_compact, format_money, format_number};
use unloop_core::tasks::{self, local_ms, TaskView};

use crate::app::{changed_later, Shared};
use crate::shell::{self, Section};
use crate::widgets::{self, esc};
use crate::{checkin_page, detail, habit_form};

/// Página-raiz de uma seção: cabeçalho + conteúdo reconstruído a cada mudança.
fn section_page(
    ctx: &Shared,
    title: &str,
    header: adw::HeaderBar,
    fill: fn(&Shared, &gtk::Box),
) -> adw::NavigationPage {
    let content = widgets::content_box();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&widgets::scroller(&content)));
    let page = adw::NavigationPage::builder().title(title).child(&toolbar).build();

    fill(ctx, &content);
    let c = ctx.clone();
    let weak = content.downgrade();
    ctx.on_refresh(move || match weak.upgrade() {
        Some(content) => {
            widgets::clear_box(&content);
            fill(&c, &content);
            true
        }
        None => false,
    });
    page
}

fn today_subtitle() -> String {
    let today = checkin::today();
    let weekday = checkin::WEEKDAY_SHORT[checkin::Datelike::weekday(&today).num_days_from_monday() as usize];
    format!("{weekday}, {}", today.format("%d/%m/%Y"))
}

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let header = shell::header("Hoje", &today_subtitle());
    let add = widgets::icon_button("list-add-symbolic", "Nova tarefa");
    header.pack_start(&add);
    let c = ctx.clone();
    add.connect_clicked(move |_| c.nav.push(&crate::tasks_page::editor(&c, None)));
    section_page(ctx, "Hoje", header, fill_today)
}

pub fn habits_page(ctx: &Shared) -> adw::NavigationPage {
    let header = shell::header("Hábitos", "");
    let add = widgets::icon_button("list-add-symbolic", "Novo hábito");
    header.pack_start(&add);
    let c = ctx.clone();
    add.connect_clicked(move |_| c.nav.push(&habit_form::page(&c, None)));
    section_page(ctx, "Hábitos", header, fill_habits)
}

/// Cartão de número do resumo (estilo TickTick).
fn stat_card(value: &str, caption: &str, icon: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.add_css_class("stat-card");
    card.set_hexpand(true);
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("stat-icon");
    image.set_halign(gtk::Align::Start);
    let v = gtk::Label::new(Some(value));
    v.add_css_class("stat-value");
    v.set_xalign(0.0);
    let cap = gtk::Label::new(Some(caption));
    cap.add_css_class("dim-label");
    cap.add_css_class("caption");
    cap.set_xalign(0.0);
    cap.set_wrap(true);
    card.append(&image);
    card.append(&v);
    card.append(&cap);
    card
}

/// Botão "Ver tudo" no cabeçalho de um grupo: abre a seção.
fn see_all(ctx: &Shared, section: Section) -> gtk::Button {
    let button = gtk::Button::with_label("Ver tudo");
    button.add_css_class("flat");
    button.add_css_class("accent");
    button.set_valign(gtk::Align::Center);
    let c = ctx.clone();
    button.connect_clicked(move |_| shell::go(&c, section));
    button
}

fn bundles(ctx: &Shared) -> Vec<HabitBundle> {
    let habits = ctx.check(ctx.db.habits()).unwrap_or_default();
    habits.iter().filter_map(|h| ctx.check(ctx.db.bundle(h.id)).flatten()).collect()
}

fn fill_today(ctx: &Shared, content: &gtk::Box) {
    let today = checkin::today();
    let now = unloop_core::now_ms();
    let all_tasks = ctx.check(ctx.db.tasks()).unwrap_or_default();
    let lists = ctx.check(ctx.db.task_lists()).unwrap_or_default();
    let due_today = tasks::filter_view(&all_tasks, TaskView::Today, today, now);
    let done_today = all_tasks
        .iter()
        .filter(|t| t.done_at.is_some_and(|d| checkin::local_date(d) == today))
        .count();
    let build: Vec<HabitBundle> = bundles(ctx).into_iter().filter(|b| b.habit.is_build()).collect();
    let scheduled: Vec<&HabitBundle> = build.iter().filter(|b| b.checkin_stats(today).scheduled_today).collect();
    let habits_done = scheduled.iter().filter(|b| b.checkin_stats(today).done_today).count();
    let focus_ms: i64 = ctx
        .check(ctx.db.focus_sessions_since(local_ms(today, 0, 0)))
        .unwrap_or_default()
        .iter()
        .map(|r| r.duration_ms)
        .sum();

    // Resumo em cartões.
    let stats = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    stats.append(&stat_card(&due_today.len().to_string(), "tarefas pendentes", "object-select-symbolic"));
    stats.append(&stat_card(&done_today.to_string(), "concluídas hoje", "emblem-ok-symbolic"));
    stats.append(&stat_card(&format!("{habits_done}/{}", scheduled.len()), "hábitos feitos", "starred-symbolic"));
    stats.append(&stat_card(&format_compact(focus_ms / 60_000), "de foco hoje", "alarm-symbolic"));
    content.append(&stats);

    let motto = ctx.setting("motto", "");
    if !motto.is_empty() {
        content.append(&widgets::label(&motto, &["affirmation", "dim-label"]));
    }

    // Tarefas de hoje (inclui atrasadas) com adição rápida.
    let summary = if due_today.is_empty() {
        "Nada pendente para hoje 🎉".to_string()
    } else {
        format!("{} pendentes (inclui atrasadas)", due_today.len())
    };
    let group = widgets::group("Tarefas", Some(summary.as_str()));
    group.set_header_suffix(Some(&see_all(ctx, Section::Tasks)));
    let list = widgets::boxed_list();
    list.append(&crate::tasks_page::quick_add(ctx, true, None));
    for t in &due_today {
        list.append(&crate::tasks_page::task_row(ctx, t, &lists, true));
    }
    group.add(&list);
    content.append(&group);

    // Hábitos de construir agendados para hoje.
    let summary = if build.is_empty() {
        "Nenhum hábito para acompanhar ainda.".to_string()
    } else if scheduled.is_empty() {
        "Hoje é folga para todos.".to_string()
    } else {
        format!("{habits_done} de {} feitos hoje", scheduled.len())
    };
    let group = widgets::group("Hábitos do dia", Some(summary.as_str()));
    group.set_header_suffix(Some(&see_all(ctx, Section::Habits)));
    if !scheduled.is_empty() {
        let list = widgets::boxed_list();
        for b in &scheduled {
            list.append(&build_row(ctx, b));
        }
        group.add(&list);
    }
    content.append(&group);
}

fn fill_habits(ctx: &Shared, content: &gtk::Box) {
    let all = bundles(ctx);
    if all.is_empty() {
        let create = widgets::pill_button("Criar meu primeiro hábito", Some("suggested-action"));
        create.set_halign(gtk::Align::Center);
        let c = ctx.clone();
        create.connect_clicked(move |_| c.nav.push(&habit_form::page(&c, None)));
        let status = adw::StatusPage::builder()
            .icon_name("starred-symbolic")
            .title("Nenhum hábito ainda")
            .description("Pare com o que te prende ou crie um hábito novo. Tudo fica só neste computador.")
            .child(&create)
            .vexpand(true)
            .build();
        content.append(&status);
        return;
    }

    let (build, quit): (Vec<HabitBundle>, Vec<HabitBundle>) = all.into_iter().partition(|b| b.habit.is_build());

    if !build.is_empty() {
        let today = checkin::today();
        let due = build.iter().filter(|b| b.checkin_stats(today).scheduled_today).count();
        let done = build
            .iter()
            .filter(|b| {
                let s = b.checkin_stats(today);
                s.scheduled_today && s.done_today
            })
            .count();
        let summary =
            if due == 0 { "Hoje é folga para todos.".to_string() } else { format!("{done} de {due} feitos hoje") };
        let group = widgets::group("Construindo", Some(summary.as_str()));
        let list = widgets::boxed_list();
        for b in &build {
            list.append(&build_row(ctx, b));
        }
        group.add(&list);
        content.append(&group);
    }

    if !quit.is_empty() {
        // Totais (atualizados a cada segundo).
        let totals = widgets::group("Desde o início", None);
        let (money_row, money_value) = widgets::value_row("Dinheiro economizado", "");
        let (time_row, time_value) = widgets::value_row("Tempo recuperado", "");
        totals.add(&money_row);
        totals.add(&time_row);
        let currency = ctx.currency();
        let habits_for_totals: Vec<_> = quit.iter().map(|b| b.habit.clone()).collect();
        let (wm, wt) = (money_value.downgrade(), time_value.downgrade());
        ctx.live(move |now| {
            let (Some(m), Some(t)) = (wm.upgrade(), wt.upgrade()) else { return false };
            let mut money = 0.0;
            let mut time = 0.0;
            for h in &habits_for_totals {
                let since = (now - h.quit_at).max(0) as f64;
                money += since * h.daily_money / 86_400_000.0;
                time += since * h.daily_minutes / 1_440.0;
            }
            m.set_label(&format_money(money, &currency));
            t.set_label(&format_compact(time.round() as i64));
            true
        });

        let list_group = widgets::group("Eu escolhi parar com", None);
        let list = widgets::boxed_list();
        for bundle in quit {
            list.append(&quit_row(ctx, bundle));
        }
        list_group.add(&list);
        content.append(&list_group);
        content.append(&totals);
    }
}

/// Linha de hábito de check: caixa (sem meta) ou progresso + botão "+" (com meta).
fn build_row(ctx: &Shared, bundle: &HabitBundle) -> adw::ActionRow {
    let habit = &bundle.habit;
    let today = checkin::today();
    let stats = bundle.checkin_stats(today);
    let amount = bundle.amount_on(today);

    let mut status = Vec::new();
    if habit.has_target() {
        status.push(format!(
            "Hoje: {} de {} {}",
            format_number(amount),
            format_number(habit.daily_target),
            habit.target_unit
        ));
    } else if stats.done_today {
        status.push("Feito hoje ✓".to_string());
    }
    if !stats.scheduled_today {
        status.push("folga hoje".to_string());
    }
    status.push(format!("🔥 {} seguidos", stats.current_streak));

    let row = adw::ActionRow::builder()
        .title(esc(&habit.name))
        .subtitle(esc(&status.join(" · ")))
        .activatable(true)
        .build();
    row.add_prefix(&widgets::avatar(habit.icon, habit.color, false));

    let id = habit.id;
    if habit.has_target() {
        let bar = gtk::ProgressBar::new();
        bar.set_fraction((amount / habit.daily_target).clamp(0.0, 1.0));
        bar.set_valign(gtk::Align::Center);
        bar.set_width_request(90);
        bar.add_css_class(&format!("bar-color-{}", unloop_core::model::color(habit.color).code));
        row.add_suffix(&bar);
        let plus = widgets::icon_button("list-add-symbolic", "Registrar progresso");
        let (c, unit, name) = (ctx.clone(), habit.target_unit.clone(), habit.name.clone());
        plus.connect_clicked(move |_| {
            let (c2, key) = (c.clone(), day_key(checkin::today()));
            widgets::ask_number(&c, &format!("Progresso: {name}"), "Quanto você fez agora? Será somado ao total de hoje.", 0.0, 1, &unit, move |delta| {
                if delta > 0.0 && c2.check(c2.db.add_progress(id, &key, delta)).is_some() {
                    c2.changed();
                }
            });
        });
        row.add_suffix(&plus);
    } else {
        let check = gtk::CheckButton::new();
        check.set_active(stats.done_today);
        check.set_valign(gtk::Align::Center);
        check.set_tooltip_text(Some("Marcar como feito hoje"));
        let c = ctx.clone();
        check.connect_toggled(move |cb| {
            let key = day_key(checkin::today());
            if c.check(c.db.set_checkin(id, &key, cb.is_active())).is_some() {
                changed_later(&c);
            }
        });
        row.add_suffix(&check);
    }

    let c = ctx.clone();
    row.connect_activated(move |_| c.nav.push(&checkin_page::page(&c, id)));
    row
}

fn quit_row(ctx: &Shared, bundle: HabitBundle) -> adw::ActionRow {
    let habit = &bundle.habit;
    let row = adw::ActionRow::builder().title(esc(&habit.name)).activatable(true).build();
    row.add_prefix(&widgets::avatar(habit.icon, habit.color, false));

    let counter = gtk::Label::new(None);
    counter.add_css_class("counter-small");
    counter.set_xalign(1.0);
    let goal = gtk::Label::new(None);
    goal.add_css_class("caption");
    goal.add_css_class("dim-label");
    goal.set_xalign(1.0);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 2);
    right.set_valign(gtk::Align::Center);
    right.append(&counter);
    right.append(&goal);
    row.add_suffix(&right);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let id = habit.id;
    let c = ctx.clone();
    row.connect_activated(move |_| c.nav.push(&detail::page(&c, id)));

    let (wc, wg) = (counter.downgrade(), goal.downgrade());
    ctx.live(move |now| {
        let (Some(counter), Some(goal)) = (wc.upgrade(), wg.upgrade()) else { return false };
        let snap = bundle.snapshot(now);
        counter.set_label(&widgets::counter_text(&bundle.habit, &snap));
        goal.set_label(&widgets::goal_text(&snap));
        true
    });
    row
}
