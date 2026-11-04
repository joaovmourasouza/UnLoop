//! Tela inicial: lema, totais, botão de foco e lista de hábitos
//! (contadores ao vivo para "parar", check do dia para "criar").

use adw::prelude::*;
use gtk::gio;
use unloop_core::calc::HabitBundle;
use unloop_core::checkin::{self, day_key};
use unloop_core::format::{format_compact, format_money, format_number};

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};
use crate::{checkin_page, detail, focus, habit_form};

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    let add = widgets::icon_button("list-add-symbolic", "Novo hábito");
    header.pack_start(&add);

    let focus_button = gtk::Button::with_label("🍅 Foco");
    focus_button.set_tooltip_text(Some("Pomodoro e estatísticas de foco (Ctrl+F)"));
    header.pack_start(&focus_button);

    let menu = gio::Menu::new();
    menu.append(Some("Foco (Pomodoro)"), Some("app.focus"));
    menu.append(Some("Configurações"), Some("app.preferences"));
    menu.append(Some("Sobre o UnLoop"), Some("app.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Menu principal")
        .build();
    header.pack_end(&menu_button);

    let content = widgets::content_box();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&widgets::scroller(&content)));

    let page = adw::NavigationPage::builder().title("UnLoop").tag("home").child(&toolbar).build();

    let c = ctx.clone();
    add.connect_clicked(move |_| c.nav.push(&habit_form::page(&c, None)));
    let c = ctx.clone();
    focus_button.connect_clicked(move |_| c.nav.push(&focus::page(&c)));

    // Mostra o tempo do foco em andamento no botão.
    let (c, weak) = (ctx.clone(), focus_button.downgrade());
    ctx.live(move |now| {
        let Some(button) = weak.upgrade() else { return false };
        let timer = c.timer.borrow();
        let label = if timer.is_idle() {
            "🍅 Foco".to_string()
        } else {
            let settings = c.pomodoro_settings.borrow();
            let paused = if timer.is_running() { "" } else { " ⏸" };
            format!("🍅 {}{paused}", widgets::clock(timer.display_ms(now, &settings)))
        };
        button.set_label(&label);
        true
    });

    rebuild(ctx, &content);
    let c = ctx.clone();
    let weak = content.downgrade();
    ctx.on_refresh(move || match weak.upgrade() {
        Some(content) => {
            rebuild(&c, &content);
            true
        }
        None => false,
    });
    page
}

fn rebuild(ctx: &Shared, content: &gtk::Box) {
    widgets::clear_box(content);
    let habits = ctx.check(ctx.db.habits()).unwrap_or_default();

    if habits.is_empty() {
        let create = widgets::pill_button("Criar meu primeiro hábito", Some("suggested-action"));
        create.set_halign(gtk::Align::Center);
        let c = ctx.clone();
        create.connect_clicked(move |_| c.nav.push(&habit_form::page(&c, None)));
        let status = adw::StatusPage::builder()
            .icon_name("emblem-ok-symbolic")
            .title("Nenhum hábito ainda")
            .description("Pare com o que te prende ou crie um hábito novo. Tudo fica só neste computador.")
            .child(&create)
            .vexpand(true)
            .build();
        content.append(&status);
        return;
    }

    let motto = ctx.setting("motto", "");
    if !motto.is_empty() {
        content.append(&widgets::label(&motto, &["title-2"]));
    }

    let bundles: Vec<HabitBundle> =
        habits.iter().filter_map(|h| ctx.check(ctx.db.bundle(h.id)).flatten()).collect();
    let (build, quit): (Vec<HabitBundle>, Vec<HabitBundle>) = bundles.into_iter().partition(|b| b.habit.is_build());

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
        let group = widgets::group("Hábitos do dia", Some(summary.as_str()));
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
