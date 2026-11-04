//! Criar ou editar um hábito (parar = contador; criar = check diário com meta opcional).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use unloop_core::checkin::{describe_mask, ALL_DAYS, WEEKDAY_LETTER, WEEKDAY_SHORT};
use unloop_core::model::{
    icon, CounterStyle, CounterType, Habit, HabitKind, MetricType, COLORS, DEFAULT_COLOR, ICONS,
};
use unloop_core::presets::{BUILD_SUGGESTIONS, HABIT_SUGGESTIONS};

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

fn suggestion_flow() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(6)
        .min_children_per_line(2)
        .column_spacing(6)
        .row_spacing(6)
        .build()
}

fn mask_of(toggles: &[gtk::ToggleButton]) -> u8 {
    toggles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.is_active())
        .fold(0u8, |m, (i, _)| m | (1 << i))
}

pub fn page(ctx: &Shared, habit: Option<Habit>) -> adw::NavigationPage {
    let editing = habit.is_some();
    let original = habit.unwrap_or_else(|| Habit::new("", unloop_core::now_ms()));
    let currency = ctx.currency();
    let quit_at = Rc::new(Cell::new(original.quit_at));
    let prefs = adw::PreferencesPage::new();

    // Tipo
    let kind_group = widgets::group("Tipo de hábito", None);
    let kind_row = combo(
        "Eu quero…",
        &HabitKind::ALL.iter().map(|k| k.label().to_string()).collect::<Vec<_>>(),
        original.kind.index() as u32,
    );
    kind_row.set_sensitive(!editing);
    if editing {
        kind_row.set_subtitle("O tipo não pode ser alterado depois de criado.");
    }
    kind_group.add(&kind_row);
    prefs.add(&kind_group);

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
        color_index(if original.color == 0 { DEFAULT_COLOR } else { original.color }),
    );

    // Check diário: dias e meta
    let schedule = widgets::group("Dias programados", Some("Só esses dias contam para a sequência."));
    let days_row = adw::ActionRow::builder().title("Dias").subtitle(describe_mask(original.schedule_days)).build();
    let days_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    days_box.set_valign(gtk::Align::Center);
    let toggles: Rc<Vec<gtk::ToggleButton>> = Rc::new(
        (0..7)
            .map(|i| {
                let t = gtk::ToggleButton::with_label(WEEKDAY_LETTER[i]);
                t.set_tooltip_text(Some(WEEKDAY_SHORT[i]));
                t.add_css_class("circular");
                t.set_active(original.schedule_days & (1 << i) != 0);
                days_box.append(&t);
                t
            })
            .collect(),
    );
    for t in toggles.iter() {
        let (container, row) = (days_box.downgrade(), days_row.downgrade());
        t.connect_toggled(move |_| {
            if let (Some(r), Some(b)) = (row.upgrade(), container.upgrade()) {
                let mut buttons = Vec::new();
                let mut child = b.first_child();
                while let Some(w) = child {
                    child = w.next_sibling();
                    if let Ok(tb) = w.downcast::<gtk::ToggleButton>() {
                        buttons.push(tb);
                    }
                }
                let mask = mask_of(&buttons);
                r.set_subtitle(&if mask == 0 { "Escolha ao menos um dia".to_string() } else { describe_mask(mask) });
            }
        });
    }
    days_row.add_suffix(&days_box);
    schedule.add(&days_row);

    let target_group = widgets::group(
        "Meta diária (opcional)",
        Some("Com meta, o dia só conta quando você atinge o valor. Ex.: 30 páginas, 50 min. Deixe 0 para só marcar feito."),
    );
    let target = spin_row("Meta por dia", "0 = só marcar feito", 100_000.0, 1, original.daily_target);
    let unit = adw::EntryRow::builder().title("Unidade (páginas, min, copos…)").build();
    unit.set_text(&original.target_unit);
    target_group.add(&target);
    target_group.add(&unit);

    // Sugestões
    let quit_suggestions = widgets::group("Sugestões", Some("Escolha um ponto de partida ou crie o seu."));
    let quit_flow = suggestion_flow();
    for &code in HABIT_SUGGESTIONS {
        let info = icon(code);
        let button = gtk::Button::with_label(&format!("{}  {}", info.emoji, info.label));
        let (n, ir) = (name.clone(), icon_row.clone());
        button.connect_clicked(move |_| {
            n.set_text(info.label);
            ir.set_selected(icon_index(code));
        });
        quit_flow.append(&button);
    }
    quit_suggestions.add(&quit_flow);

    let build_suggestions = widgets::group("Sugestões", Some("Meta e dias já vêm preenchidos; ajuste à vontade."));
    let build_flow = suggestion_flow();
    for s in BUILD_SUGGESTIONS {
        let info = icon(s.icon);
        let text = if s.target > 0.0 {
            format!("{}  {} ({} {})", info.emoji, info.label, unloop_core::format::format_number(s.target), s.unit)
        } else {
            format!("{}  {}", info.emoji, info.label)
        };
        let button = gtk::Button::with_label(&text);
        let (n, ir, tg, un, all) = (name.clone(), icon_row.clone(), target.clone(), unit.clone(), toggles.clone());
        button.connect_clicked(move |_| {
            n.set_text(info.label);
            ir.set_selected(icon_index(s.icon));
            tg.set_value(s.target);
            un.set_text(s.unit);
            for (i, t) in all.iter().enumerate() {
                t.set_active(s.days & (1 << i) != 0);
            }
        });
        build_flow.append(&button);
    }
    build_suggestions.add(&build_flow);

    if !editing {
        prefs.add(&quit_suggestions);
        prefs.add(&build_suggestions);
    }

    let identity = widgets::group("Identificação", None);
    identity.add(&name);
    identity.add(&icon_row);
    identity.add(&color_row);
    prefs.add(&identity);

    // Início
    let start = widgets::group("Início", None);
    let quit_row = adw::ActionRow::builder().title("Última vez").subtitle(widgets::fmt_datetime(original.quit_at)).build();
    let change = gtk::Button::with_label("Alterar");
    change.set_valign(gtk::Align::Center);
    quit_row.add_suffix(&change);
    {
        let (c, q, row) = (ctx.clone(), quit_at.clone(), quit_row.downgrade());
        change.connect_clicked(move |_| {
            let (q, row) = (q.clone(), row.clone());
            widgets::pick_datetime(&c, "Escolha a data", q.get(), None, Some(unloop_core::now_ms()), move |ms| {
                q.set(ms);
                if let Some(r) = row.upgrade() {
                    r.set_subtitle(&widgets::fmt_datetime(ms));
                }
            });
        });
    }
    start.add(&quit_row);
    prefs.add(&start);
    prefs.add(&schedule);
    prefs.add(&target_group);

    // Custos (só para parar)
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

    // Mostra só o que vale para o tipo escolhido.
    let apply_kind = {
        let (qs, bs, sc, tg, co, ct, row) = (
            quit_suggestions.clone(),
            build_suggestions.clone(),
            schedule.clone(),
            target_group.clone(),
            costs.clone(),
            counter.clone(),
            quit_row.clone(),
        );
        move |kind: HabitKind| {
            let build = kind == HabitKind::Build;
            qs.set_visible(!build);
            bs.set_visible(build);
            sc.set_visible(build);
            tg.set_visible(build);
            co.set_visible(!build);
            ct.set_visible(!build);
            row.set_title(if build { "Começa em" } else { "Última vez" });
        }
    };
    apply_kind(original.kind);
    kind_row.connect_selected_notify(move |r| apply_kind(HabitKind::from_index(r.selected() as usize)));

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
        let kind = if editing { original.kind } else { HabitKind::from_index(kind_row.selected() as usize) };
        let mask = mask_of(&toggles);
        if kind == HabitKind::Build && mask == 0 {
            c.toast("Escolha ao menos um dia da semana.");
            return;
        }
        let mut h = original.clone();
        h.name = name_text;
        h.kind = kind;
        h.icon = ICONS.get(icon_row.selected() as usize).map_or(ICONS[0].code, |i| i.code);
        h.color = COLORS.get(color_row.selected() as usize).map_or(DEFAULT_COLOR, |co| co.code);
        h.quit_at = quit_at.get().min(unloop_core::now_ms());
        if kind == HabitKind::Build {
            h.schedule_days = mask;
            h.daily_target = target.value();
            h.target_unit = unit.text().trim().to_string();
            h.daily_money = 0.0;
            h.daily_minutes = 0.0;
            h.daily_frequency = 0.0;
        } else {
            h.schedule_days = ALL_DAYS;
            h.daily_money = money.value();
            h.daily_minutes = minutes.value();
            h.daily_frequency = frequency.value();
            h.preferred_metric = MetricType::from_index(metric.selected() as usize);
            h.counter_type = CounterType::from_index(counter_type.selected() as usize);
            h.counter_style = CounterStyle::from_index(counter_style.selected() as usize);
        }

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
