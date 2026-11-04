//! Componentes e diálogos reutilizáveis.

use adw::prelude::*;
use gtk::glib;
use unloop_core::calc::HabitSnapshot;
use unloop_core::format::{format_compact, format_counter};
use unloop_core::model::{color, icon, CounterType, Habit};

use crate::app::Shared;

/// Escapa texto do usuário para títulos/subtítulos (que interpretam Pango markup).
pub fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

pub fn clear_box(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

pub fn content_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 18);
    b.set_margin_top(18);
    b.set_margin_bottom(24);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b
}

/// Rolagem vertical com largura limitada (padrão GNOME).
pub fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder().maximum_size(820).child(child).build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build()
}

pub fn boxed_list() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list
}

pub fn group(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    if let Some(d) = description {
        g.set_description(Some(d));
    }
    g
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_justify(gtk::Justification::Center);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// Linha com um valor à direita. Retorna a linha e o rótulo do valor.
pub fn value_row(title: &str, value: &str) -> (adw::ActionRow, gtk::Label) {
    let row = adw::ActionRow::builder().title(esc(title)).build();
    let v = gtk::Label::new(Some(value));
    v.add_css_class("numeric");
    v.set_selectable(true);
    row.add_suffix(&v);
    (row, v)
}

pub fn icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon_name);
    b.set_tooltip_text(Some(tooltip));
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b
}

pub fn pill_button(text: &str, class: Option<&str>) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("pill");
    if let Some(c) = class {
        b.add_css_class(c);
    }
    b
}

/// Círculo colorido com o emoji do hábito.
pub fn avatar(icon_code: i64, color_code: i64, large: bool) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("avatar");
    if large {
        b.add_css_class("large");
    }
    b.add_css_class(&format!("habit-color-{}", color(color_code).code));
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    let l = gtk::Label::new(Some(icon(icon_code).emoji));
    l.set_vexpand(true);
    l.set_hexpand(true);
    l.set_valign(gtk::Align::Center);
    l.set_halign(gtk::Align::Center);
    b.append(&l);
    b
}

// ------------------------------------------------------------ Datas (local)

fn fmt_local(ms: i64, pattern: &str) -> String {
    glib::DateTime::from_unix_local(ms.div_euclid(1000))
        .ok()
        .and_then(|d| d.format(pattern).ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn fmt_date(ms: i64) -> String {
    fmt_local(ms, "%d/%m/%Y")
}

pub fn fmt_datetime(ms: i64) -> String {
    fmt_local(ms, "%d/%m/%Y às %H:%M")
}

pub fn fmt_time(ms: i64) -> String {
    fmt_local(ms, "%H:%M")
}

pub fn file_stamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y%m%d-%H%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "backup".into())
}

/// "mm:ss" (minutos podem passar de 60 no cronômetro).
pub fn clock(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Gráfico de barras verticais simples. `values` = (rótulo, valor em ms).
pub fn bar_chart(values: &[(String, i64)], color_class: Option<&str>) -> gtk::Box {
    let chart = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    chart.set_homogeneous(true);
    chart.set_height_request(150);
    let max = values.iter().map(|v| v.1).max().unwrap_or(0).max(1);
    let step = if values.len() > 12 { (values.len() + 7) / 8 } else { 1 };
    for (i, (label, value)) in values.iter().enumerate() {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let bar = gtk::ProgressBar::new();
        bar.set_orientation(gtk::Orientation::Vertical);
        bar.set_inverted(true);
        bar.set_vexpand(true);
        bar.set_halign(gtk::Align::Center);
        bar.set_fraction(*value as f64 / max as f64);
        bar.add_css_class("thick");
        if let Some(c) = color_class {
            bar.add_css_class(c);
        }
        column.set_tooltip_text(Some(&format!("{label}: {}", format_compact(*value))));
        column.append(&bar);
        let caption = gtk::Label::new(Some(if i % step == 0 { label.as_str() } else { "" }));
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        column.append(&caption);
        chart.append(&column);
    }
    chart
}

// --------------------------------------------------------- Textos do contador

pub fn counter_text(h: &Habit, s: &HabitSnapshot) -> String {
    if h.counter_type == CounterType::Remaining && s.goal.is_some() {
        format_counter(s.goal_remaining(), h.counter_style)
    } else {
        format_counter(s.elapsed(), h.counter_style)
    }
}

pub fn counter_caption(h: &Habit, s: &HabitSnapshot) -> &'static str {
    if h.counter_type == CounterType::Remaining && s.goal.is_some() {
        "Faltam para a meta"
    } else {
        "Tempo limpo"
    }
}

pub fn goal_text(s: &HabitSnapshot) -> String {
    match &s.goal {
        Some(g) => format!(
            "Meta: {} · {}% · faltam {}",
            g.name,
            (s.goal_progress() * 100.0).floor(),
            format_compact(s.goal_remaining())
        ),
        None => "Todas as metas alcançadas".to_string(),
    }
}

// ----------------------------------------------------------------- Diálogos

fn dialog(ctx: &Shared, heading: &str, body: &str) -> adw::MessageDialog {
    adw::MessageDialog::new(Some(&ctx.window), Some(heading), if body.is_empty() { None } else { Some(body) })
}

pub fn info(ctx: &Shared, heading: &str, body: &str) {
    let d = dialog(ctx, heading, body);
    d.add_response("ok", "Fechar");
    d.set_default_response(Some("ok"));
    d.set_close_response("ok");
    d.present();
}

pub fn confirm(
    ctx: &Shared,
    heading: &str,
    body: &str,
    ok_label: &str,
    destructive: bool,
    on_ok: impl Fn() + 'static,
) {
    let d = dialog(ctx, heading, body);
    d.add_responses(&[("cancel", "Cancelar"), ("ok", ok_label)]);
    d.set_response_appearance(
        "ok",
        if destructive { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested },
    );
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, response| {
        if response == "ok" {
            on_ok();
        }
    });
    d.present();
}

pub fn ask_text(ctx: &Shared, heading: &str, initial: &str, placeholder: &str, on_ok: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder()
        .text(initial)
        .placeholder_text(placeholder)
        .activates_default(true)
        .width_chars(40)
        .build();
    let d = dialog(ctx, heading, "");
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", "Cancelar"), ("ok", "Salvar")]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, response| {
        if response == "ok" {
            let text = entry.text().trim().to_string();
            if !text.is_empty() {
                on_ok(text);
            }
        }
    });
    d.present();
}

/// Nome + valor numérico (recompensa com preço, investimento com minutos).
pub fn ask_name_number(
    ctx: &Shared,
    heading: &str,
    number_label: &str,
    initial_name: &str,
    initial_number: f64,
    digits: u32,
    on_ok: impl Fn(String, f64) + 'static,
) {
    let entry = gtk::Entry::builder()
        .text(initial_name)
        .placeholder_text("Nome")
        .activates_default(true)
        .width_chars(36)
        .build();
    let spin = gtk::SpinButton::with_range(0.0, 1_000_000.0, 1.0);
    spin.set_digits(digits);
    spin.set_value(initial_number);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let caption = gtk::Label::new(Some(number_label));
    caption.set_hexpand(true);
    caption.set_xalign(0.0);
    row.append(&caption);
    row.append(&spin);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&entry);
    content.append(&row);

    let d = dialog(ctx, heading, "");
    d.set_extra_child(Some(&content));
    d.add_responses(&[("cancel", "Cancelar"), ("ok", "Salvar")]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let c = ctx.clone();
    d.connect_response(None, move |_, response| {
        if response != "ok" {
            return;
        }
        let name = entry.text().trim().to_string();
        let value = spin.value();
        if name.is_empty() || value <= 0.0 {
            c.toast("Informe um nome e um valor maior que zero.");
            return;
        }
        on_ok(name, value);
    });
    d.present();
}

/// Pede um número (aceita zero).
pub fn ask_number(
    ctx: &Shared,
    heading: &str,
    body: &str,
    initial: f64,
    digits: u32,
    suffix: &str,
    on_ok: impl Fn(f64) + 'static,
) {
    let spin = gtk::SpinButton::with_range(0.0, 1_000_000.0, 1.0);
    spin.set_digits(digits);
    spin.set_value(initial);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    row.append(&spin);
    if !suffix.is_empty() {
        row.append(&gtk::Label::new(Some(suffix)));
    }
    let d = dialog(ctx, heading, body);
    d.set_extra_child(Some(&row));
    d.add_responses(&[("cancel", "Cancelar"), ("ok", "Salvar")]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, response| {
        if response == "ok" {
            on_ok(spin.value());
        }
    });
    d.present();
}

/// Escolhe data (calendário) e hora. O resultado é limitado a [min, max].
pub fn pick_datetime(
    ctx: &Shared,
    heading: &str,
    initial_ms: i64,
    min_ms: Option<i64>,
    max_ms: Option<i64>,
    on_ok: impl Fn(i64) + 'static,
) {
    let calendar = gtk::Calendar::new();
    let initial = glib::DateTime::from_unix_local(initial_ms.div_euclid(1000)).ok();
    if let Some(dt) = &initial {
        calendar.select_day(dt);
    }
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    hour.set_value(initial.as_ref().map_or(0, |d| d.hour()) as f64);
    minute.set_value(initial.as_ref().map_or(0, |d| d.minute()) as f64);
    hour.set_wrap(true);
    minute.set_wrap(true);

    let time_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    time_row.set_halign(gtk::Align::Center);
    time_row.append(&gtk::Label::new(Some("Hora")));
    time_row.append(&hour);
    time_row.append(&gtk::Label::new(Some(":")));
    time_row.append(&minute);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&calendar);
    content.append(&time_row);

    let d = dialog(ctx, heading, "");
    d.set_extra_child(Some(&content));
    d.add_responses(&[("cancel", "Cancelar"), ("ok", "Confirmar")]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, response| {
        if response != "ok" {
            return;
        }
        let date = calendar.date();
        let Ok(chosen) = glib::DateTime::from_local(
            date.year(),
            date.month(),
            date.day_of_month(),
            hour.value_as_int(),
            minute.value_as_int(),
            0.0,
        ) else {
            return;
        };
        let mut ms = chosen.to_unix() * 1000;
        if let Some(max) = max_ms {
            ms = ms.min(max);
        }
        if let Some(min) = min_ms {
            ms = ms.max(min);
        }
        on_ok(ms);
    });
    d.present();
}
