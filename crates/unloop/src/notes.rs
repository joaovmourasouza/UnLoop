//! Notas do hábito: gatilhos, o que ajuda, notas livres e análises (suas ou do Claude).

use adw::prelude::*;
use unloop_core::model::{join_tags, parse_tags, HabitNote, NoteKind};

use crate::app::Shared;
use crate::widgets::{self, esc};

const ORDER: [NoteKind; 4] = [NoteKind::Trigger, NoteKind::Helper, NoteKind::Note, NoteKind::Insight];

fn preview(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        format!("{}…", one_line.chars().take(max).collect::<String>())
    } else {
        one_line
    }
}

/// Conteúdo da aba "Notas" de um hábito.
pub fn tab(ctx: &Shared, habit_id: i64, b: &gtk::Box) {
    let notes = ctx.check(ctx.db.notes(Some(habit_id), None)).unwrap_or_default();
    b.append(&widgets::label(
        "Anote o que te leva a fazer e o que te ajuda a evitar. O Claude também pode ler, \
         analisar e escrever aqui pelo `unloop-cli` (as notas dele aparecem com 🤖).",
        &["dim-label"],
    ));

    for kind in ORDER {
        let group = widgets::group(&format!("{} {}", kind.emoji(), kind.label()), Some(kind.description()));
        let add = widgets::icon_button("list-add-symbolic", "Nova nota");
        let c = ctx.clone();
        add.connect_clicked(move |_| c.nav.push(&editor(&c, habit_id, None, kind)));
        group.set_header_suffix(Some(&add));

        let items: Vec<&HabitNote> = notes.iter().filter(|n| n.kind == kind).collect();
        if items.is_empty() {
            let empty = adw::ActionRow::builder()
                .title("Nada ainda")
                .subtitle(match kind {
                    NoteKind::Trigger => "Ex.: \"Depois do almoço, com sono, bate vontade de refrigerante.\"",
                    NoteKind::Helper => "Ex.: \"Deixar água com gás gelada na geladeira.\"",
                    NoteKind::Note => "Qualquer coisa que valha lembrar.",
                    NoteKind::Insight => "Peça ao Claude: \"analise meus gatilhos de refrigerante\".",
                })
                .build();
            empty.add_css_class("dim-label");
            group.add(&empty);
        } else {
            let list = widgets::boxed_list();
            for n in items {
                let title = if n.title.is_empty() { preview(&n.body, 60) } else { n.title.clone() };
                let mut meta = Vec::new();
                if !n.body.is_empty() && !n.title.is_empty() {
                    meta.push(preview(&n.body, 140));
                }
                if !n.tags.is_empty() {
                    meta.push(n.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "));
                }
                meta.push(format!(
                    "{}{}",
                    widgets::fmt_date(n.updated_at),
                    if n.by_claude() { " · 🤖 Claude" } else { "" }
                ));
                let row = adw::ActionRow::builder()
                    .title(esc(&title))
                    .subtitle(esc(&meta.join(" · ")))
                    .subtitle_lines(3)
                    .activatable(true)
                    .build();
                let (c, note) = (ctx.clone(), n.clone());
                row.connect_activated(move |_| c.nav.push(&editor(&c, habit_id, Some(note.clone()), note.kind)));
                let delete = widgets::icon_button("user-trash-symbolic", "Apagar nota");
                let (c, id) = (ctx.clone(), n.id);
                delete.connect_clicked(move |_| {
                    let c2 = c.clone();
                    widgets::confirm(&c, "Apagar nota?", "Esta nota será apagada permanentemente.", "Apagar", true, move || {
                        if c2.check(c2.db.delete_note(id)).is_some() {
                            c2.changed();
                        }
                    });
                });
                row.add_suffix(&delete);
                list.append(&row);
            }
            group.add(&list);
        }
        b.append(&group);
    }
}

/// Editor de nota (criar ou editar).
pub fn editor(ctx: &Shared, habit_id: i64, note: Option<HabitNote>, kind: NoteKind) -> adw::NavigationPage {
    let editing = note.is_some();
    let original = note.unwrap_or_else(|| HabitNote::new(habit_id, kind, "", ""));
    let prefs = adw::PreferencesPage::new();

    let group = widgets::group("Nota", None);
    let kinds: Vec<String> = ORDER.iter().map(|k| format!("{} {}", k.emoji(), k.label())).collect();
    let kind_refs: Vec<&str> = kinds.iter().map(String::as_str).collect();
    let kind_row = adw::ComboRow::builder().title("Tipo").model(&gtk::StringList::new(&kind_refs)).build();
    kind_row.set_selected(ORDER.iter().position(|k| *k == original.kind).unwrap_or(0) as u32);
    let title = adw::EntryRow::builder().title("Título").build();
    title.set_text(&original.title);
    let tags = adw::EntryRow::builder().title("Tags (separadas por vírgula)").build();
    tags.set_text(&join_tags(&original.tags));
    group.add(&kind_row);
    group.add(&title);
    group.add(&tags);
    if original.by_claude() {
        let source = adw::ActionRow::builder().title("🤖 Escrita pelo Claude").build();
        group.add(&source);
    }
    prefs.add(&group);

    let body_group = widgets::group("Texto", Some("Markdown simples é aceito."));
    let body = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    body.buffer().set_text(&original.body);
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&body));
    frame.set_height_request(260);
    frame.add_css_class("view");
    body_group.add(&frame);
    prefs.add(&body_group);

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar nota" } else { "Nova nota" })
        .child(&toolbar)
        .build();

    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let buffer = body.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false).to_string();
        let mut n = original.clone();
        n.kind = ORDER.get(kind_row.selected() as usize).copied().unwrap_or(NoteKind::Note);
        n.title = title.text().trim().to_string();
        n.body = text.trim().to_string();
        n.tags = parse_tags(&tags.text());
        if n.title.is_empty() && n.body.is_empty() {
            c.toast("Escreva um título ou um texto.");
            return;
        }
        let ok = if editing { c.check(c.db.update_note(&n)).is_some() } else { c.check(c.db.insert_note(&n)).is_some() };
        if ok {
            c.changed();
            c.nav.pop();
        }
    });
    page
}
