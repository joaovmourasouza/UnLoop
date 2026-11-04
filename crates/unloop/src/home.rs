//! Tela inicial: lema, totais e lista de hábitos com contadores ao vivo.

use adw::prelude::*;
use gtk::gio;
use unloop_core::calc::HabitBundle;
use unloop_core::format::{format_compact, format_money};

use crate::app::Shared;
use crate::widgets::{self, esc};
use crate::{detail, habit_form};

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    let add = widgets::icon_button("list-add-symbolic", "Novo hábito");
    header.pack_start(&add);

    let menu = gio::Menu::new();
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
            .description("Escolha o que você quer deixar para trás. Tudo fica só neste computador.")
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

    // Totais (atualizados a cada segundo).
    let totals = widgets::group("Desde o início", None);
    let (money_row, money_value) = widgets::value_row("Dinheiro economizado", "");
    let (time_row, time_value) = widgets::value_row("Tempo recuperado", "");
    totals.add(&money_row);
    totals.add(&time_row);
    content.append(&totals);
    let currency = ctx.currency();
    let habits_for_totals = habits.clone();
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
    for bundle in bundles {
        list.append(&habit_row(ctx, bundle));
    }
    list_group.add(&list);
    content.append(&list_group);
}

fn habit_row(ctx: &Shared, bundle: HabitBundle) -> adw::ActionRow {
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
