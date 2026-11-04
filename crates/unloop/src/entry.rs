//! Criar ou editar uma entrada do diário (recaída ou anotação).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use unloop_core::model::DiaryEntry;
use unloop_core::units::MS_PER_MINUTE;

use crate::app::Shared;
use crate::widgets;

pub fn page(ctx: &Shared, habit_id: i64, entry: Option<DiaryEntry>, initial_reset: bool) -> adw::NavigationPage {
    let editing = entry.is_some();
    let original = entry.unwrap_or_else(|| DiaryEntry {
        is_reset: initial_reset,
        ..DiaryEntry::reset(habit_id, unloop_core::now_ms())
    });
    let when = Rc::new(Cell::new(original.occurred_at));
    let prefs = adw::PreferencesPage::new();

    let event = widgets::group("O que aconteceu", None);
    let when_row = adw::ActionRow::builder()
        .title("Quando")
        .subtitle(widgets::fmt_datetime(original.occurred_at))
        .build();
    let change = gtk::Button::with_label("Alterar");
    change.set_valign(gtk::Align::Center);
    when_row.add_suffix(&change);
    {
        let (c, w, row) = (ctx.clone(), when.clone(), when_row.downgrade());
        change.connect_clicked(move |_| {
            let (w, row) = (w.clone(), row.clone());
            widgets::pick_datetime(&c, "Quando aconteceu?", w.get(), None, Some(unloop_core::now_ms()), move |ms| {
                w.set(ms);
                if let Some(r) = row.upgrade() {
                    r.set_subtitle(&widgets::fmt_datetime(ms));
                }
            });
        });
    }
    let reset = adw::SwitchRow::builder()
        .title("Foi uma recaída")
        .subtitle("Recaídas reiniciam o contador a partir da data escolhida. Desligue para registrar só uma anotação ou um gasto.")
        .active(original.is_reset)
        .build();
    event.add(&when_row);
    event.add(&reset);
    prefs.add(&event);

    let costs = widgets::group("Gastos (opcional)", None);
    let money = adw::SpinRow::with_range(0.0, 1_000_000.0, 1.0);
    money.set_title("Dinheiro gasto");
    money.set_subtitle(&ctx.currency());
    money.set_digits(2);
    money.set_value(original.money_spent);
    let minutes = adw::SpinRow::with_range(0.0, 100_000.0, 5.0);
    minutes.set_title("Tempo gasto");
    minutes.set_subtitle("minutos");
    minutes.set_value(original.time_spent_ms as f64 / MS_PER_MINUTE as f64);
    costs.add(&money);
    costs.add(&minutes);
    prefs.add(&costs);

    let notes = widgets::group(
        "Pensamentos e notas",
        Some("O que você estava sentindo? O que levou a isso? Anotar ajuda a reconhecer gatilhos."),
    );
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    text.buffer().set_text(original.note.as_deref().unwrap_or(""));
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&text));
    frame.set_height_request(160);
    frame.add_css_class("view");
    notes.add(&frame);
    prefs.add(&notes);

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar entrada" } else { "Nova entrada" })
        .child(&toolbar)
        .build();

    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let buffer = text.buffer();
        let (start, end) = buffer.bounds();
        let note = buffer.text(&start, &end, false).to_string();
        let e = DiaryEntry {
            occurred_at: when.get().min(unloop_core::now_ms()),
            money_spent: money.value(),
            time_spent_ms: (minutes.value() * MS_PER_MINUTE as f64).round() as i64,
            is_reset: reset.is_active(),
            note: if note.trim().is_empty() { None } else { Some(note) },
            ..original.clone()
        };
        let ok = if editing {
            c.check(c.db.update_entry(&e)).is_some()
        } else {
            c.check(c.db.insert_entry(&e)).is_some()
        };
        if ok {
            c.changed();
            c.nav.pop();
            c.toast(if e.is_reset { "Recaída registrada. Recomeçar também é coragem." } else { "Entrada salva." });
        }
    });

    page
}
