//! Criar ou editar um hábito.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use unloop_core::model::{icon, CounterStyle, CounterType, Habit, MetricType, COLORS, DEFAULT_COLOR, ICONS};
use unloop_core::presets::HABIT_SUGGESTIONS;

use crate::app::Shared;
use crate::widgets;

fn icon_index(code: i64) -> u32 {
    ICONS.iter().position(|i| i.code == code).unwrap_or(0) as u32
}

fn color_index(code: i64) -> u32 {
    COLORS.iter().position(|c| c.code == code).unwrap_or(0) as u32
}

fn spin_row(title: &str, subtitle: &str, max: f64, digits: u32, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(0.0, max, 1.0);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_digits(digits);
    row.set_value(value);
    row
}

fn combo(title: &str, items: &[String], selected: u32) -> adw::ComboRow {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(&refs)).build();
    row.set_selected(selected);
    row
}

pub fn page(ctx: &Shared, habit: Option<Habit>) -> adw::NavigationPage {
    let editing = habit.is_some();
    let original = habit.unwrap_or_else(|| {
        let mut h = Habit::new("", unloop_core::now_ms());
        h.color = DEFAULT_COLOR;
        h
    });
    let currency = ctx.currency();
    let quit_at = Rc::new(Cell::new(original.quit_at));

    let prefs = adw::PreferencesPage::new();

    // Identificação
    let name = adw::EntryRow::builder().title("Nome do hábito").build();
    name.set_text(&original.name);
    let icon_row = combo(
        "Ícone",
        &ICONS.iter().map(|i| format!("{}  {}", i.emoji, i.label)).collect::<Vec<_>>(),
        icon_index(original.icon),
    );
    let color_row = combo(
        "Cor",
        &COLORS.iter().map(|c| c.label.to_string()).collect::<Vec<_>>(),
        color_index(original.color),
    );

    if !editing {
        let suggestions = widgets::group("Sugestões", Some("Escolha um ponto de partida ou crie o seu."));
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(6)
            .min_children_per_line(2)
            .column_spacing(6)
            .row_spacing(6)
            .build();
        for &code in HABIT_SUGGESTIONS {
            let info = icon(code);
            let button = gtk::Button::with_label(&format!("{}  {}", info.emoji, info.label));
            let (n, ir) = (name.clone(), icon_row.clone());
            button.connect_clicked(move |_| {
                n.set_text(info.label);
                ir.set_selected(icon_index(code));
            });
            flow.append(&button);
        }
        suggestions.add(&flow);
        prefs.add(&suggestions);
    }

    let identity = widgets::group("Identificação", None);
    identity.add(&name);
    identity.add(&icon_row);
    identity.add(&color_row);
    prefs.add(&identity);

    // Início
    let start = widgets::group("Início da jornada", Some("A data da última vez. O contador começa daqui."));
    let quit_row = adw::ActionRow::builder()
        .title("Última vez")
        .subtitle(widgets::fmt_datetime(original.quit_at))
        .build();
    let change = gtk::Button::with_label("Alterar");
    change.set_valign(gtk::Align::Center);
    quit_row.add_suffix(&change);
    {
        let (c, q, row) = (ctx.clone(), quit_at.clone(), quit_row.downgrade());
        change.connect_clicked(move |_| {
            let (q, row) = (q.clone(), row.clone());
            widgets::pick_datetime(&c, "Quando foi a última vez?", q.get(), None, Some(unloop_core::now_ms()), move |ms| {
                q.set(ms);
                if let Some(r) = row.upgrade() {
                    r.set_subtitle(&widgets::fmt_datetime(ms));
                }
            });
        });
    }
    start.add(&quit_row);
    prefs.add(&start);

    // Custos
    let costs = widgets::group(
        "Quanto o hábito custava por dia",
        Some("Estimativas. Servem para calcular o dinheiro e o tempo que você recupera."),
    );
    let money = spin_row("Dinheiro por dia", &currency, 1_000_000.0, 2, original.daily_money);
    let minutes = spin_row("Tempo por dia", "minutos", 1_440.0, 0, original.daily_minutes);
    let frequency = spin_row("Vezes por dia", "", 1_000.0, 1, original.daily_frequency);
    let metric = combo(
        "Destacar",
        &MetricType::ALL.iter().map(|m| m.label().to_string()).collect::<Vec<_>>(),
        original.preferred_metric.index() as u32,
    );
    costs.add(&money);
    costs.add(&minutes);
    costs.add(&frequency);
    costs.add(&metric);
    prefs.add(&costs);

    // Contador
    let counter = widgets::group("Contador", None);
    let counter_type = combo(
        "Tipo",
        &CounterType::ALL.iter().map(|t| t.label().to_string()).collect::<Vec<_>>(),
        original.counter_type.index() as u32,
    );
    let sample = 3 * 86_400_000 + 4 * 3_600_000 + 5 * 60_000 + 6_000;
    let counter_style = combo(
        "Formato",
        &CounterStyle::ALL
            .iter()
            .map(|s| format!("{} ({})", s.label(), unloop_core::format::format_counter(sample, *s)))
            .collect::<Vec<_>>(),
        original.counter_style.index() as u32,
    );
    counter.add(&counter_type);
    counter.add(&counter_style);
    prefs.add(&counter);

    // Cabeçalho
    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));

    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar hábito" } else { "Novo hábito" })
        .child(&toolbar)
        .build();

    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let name_text = name.text().trim().to_string();
        if name_text.is_empty() {
            c.toast("Dê um nome ao hábito.");
            return;
        }
        let mut h = original.clone();
        h.name = name_text;
        h.icon = ICONS.get(icon_row.selected() as usize).map_or(ICONS[0].code, |i| i.code);
        h.color = COLORS.get(color_row.selected() as usize).map_or(DEFAULT_COLOR, |co| co.code);
        h.quit_at = quit_at.get().min(unloop_core::now_ms());
        h.daily_money = money.value();
        h.daily_minutes = minutes.value();
        h.daily_frequency = frequency.value();
        h.preferred_metric = MetricType::from_index(metric.selected() as usize);
        h.counter_type = CounterType::from_index(counter_type.selected() as usize);
        h.counter_style = CounterStyle::from_index(counter_style.selected() as usize);

        let ok = if editing {
            c.check(c.db.update_habit(&h)).is_some()
        } else {
            c.check(c.db.insert_habit(&h, true)).is_some()
        };
        if ok {
            c.changed();
            c.nav.pop();
            c.toast(if editing { "Hábito atualizado." } else { "Hábito criado. Um passo de cada vez!" });
        }
    });

    page
}
