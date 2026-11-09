//! Tarefas no estilo TickTick: coluna de listas (Hoje, 7 dias, Caixa de entrada,
//! Todas, Concluídas e as listas do usuário, com contadores) + tarefas da lista
//! escolhida. Também o editor de tarefa.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use unloop_core::checkin::{self, local_date};
use unloop_core::model::{color, join_tags, parse_tags, Priority, Task, TaskList};
use unloop_core::tasks::{self, local_ms, TaskView};

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};

const VIEWS: [TaskView; 5] = [TaskView::Today, TaskView::Next7, TaskView::Inbox, TaskView::All, TaskView::Done];

/// O que está aberto na coluna de listas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sel {
    View(TaskView),
    List(i64),
}

struct State {
    sel: Cell<Sel>,
    /// Item de cada linha da coluna (`None` = linha de título, não clicável).
    rows: RefCell<Vec<Option<Sel>>>,
}

/// Referências fracas aos widgets que mudam a cada atualização.
struct Ui {
    side: glib::WeakRef<gtk::ListBox>,
    content: glib::WeakRef<gtk::Box>,
    title: glib::WeakRef<adw::WindowTitle>,
    manage: glib::WeakRef<gtk::Button>,
}

fn view_icon(view: TaskView) -> &'static str {
    match view {
        TaskView::Today => "weather-clear-symbolic",
        TaskView::Next7 => "x-office-calendar-symbolic",
        TaskView::Inbox => "mail-inbox-symbolic",
        TaskView::Done => "emblem-ok-symbolic",
        _ => "view-list-symbolic",
    }
}

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let state = Rc::new(State { sel: Cell::new(Sel::View(TaskView::Today)), rows: RefCell::new(Vec::new()) });

    // Coluna de listas.
    let side = gtk::ListBox::new();
    side.add_css_class("navigation-sidebar");
    side.set_selection_mode(gtk::SelectionMode::Single);
    let side_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&side)
        .build();
    let new_list_inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    new_list_inner.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    new_list_inner.append(&gtk::Label::new(Some("Nova lista")));
    let new_list = gtk::Button::builder().child(&new_list_inner).build();
    new_list.add_css_class("flat");
    new_list.set_margin_start(8);
    new_list.set_margin_end(8);
    new_list.set_margin_bottom(8);
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&side_scroll);
    side_box.append(&new_list);
    let side_header = adw::HeaderBar::new();
    side_header.set_title_widget(Some(&adw::WindowTitle::new("Tarefas", "")));
    let side_toolbar = adw::ToolbarView::new();
    side_toolbar.add_top_bar(&side_header);
    side_toolbar.set_content(Some(&side_box));

    // Conteúdo da lista escolhida.
    let content = widgets::content_box();
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let toggle = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text("Mostrar listas").build();
    header.pack_start(&toggle);
    let add = widgets::icon_button("list-add-symbolic", "Nova tarefa");
    header.pack_start(&add);
    let manage = widgets::icon_button("view-more-symbolic", "Renomear ou apagar a lista");
    header.pack_end(&manage);
    let main_toolbar = adw::ToolbarView::new();
    main_toolbar.add_top_bar(&header);
    main_toolbar.set_content(Some(&widgets::scroller(&content)));

    let split = adw::OverlaySplitView::builder()
        .sidebar(&side_toolbar)
        .content(&main_toolbar)
        .min_sidebar_width(220.0)
        .max_sidebar_width(280.0)
        .build();
    split.bind_property("show-sidebar", &toggle, "active").bidirectional().sync_create().build();
    split.bind_property("collapsed", &toggle, "visible").sync_create().build();

    let bin = adw::BreakpointBin::builder().width_request(360).height_request(300).child(&split).build();
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 640sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        bin.add_breakpoint(breakpoint);
    }
    let page = adw::NavigationPage::builder().title("Tarefas").child(&bin).build();

    let ui = Rc::new(Ui {
        side: side.downgrade(),
        content: content.downgrade(),
        title: title.downgrade(),
        manage: manage.downgrade(),
    });

    // Escolher uma lista na coluna.
    let (c, st, u, ws) = (ctx.clone(), state.clone(), ui.clone(), split.downgrade());
    side.connect_row_activated(move |_, row| {
        let picked = st.rows.borrow().get(row.index().max(0) as usize).copied().flatten();
        let Some(sel) = picked else { return };
        st.sel.set(sel);
        if let Some(split) = ws.upgrade() {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        }
        // Reconstruir depois do sinal: a linha clicada é removida na reconstrução.
        let (c, st, u) = (c.clone(), st.clone(), u.clone());
        glib::idle_add_local_once(move || {
            render(&c, &st, &u);
        });
    });

    let (c, st) = (ctx.clone(), state.clone());
    add.connect_clicked(move |_| {
        let mut task = Task::new("");
        match st.sel.get() {
            Sel::List(id) => task.list_id = Some(id),
            Sel::View(TaskView::Today | TaskView::Next7) => task.due_at = Some(local_ms(checkin::today(), 12, 0)),
            Sel::View(_) => {}
        }
        c.nav.push(&editor(&c, Some(task)));
    });

    let (c, st) = (ctx.clone(), state.clone());
    new_list.connect_clicked(move |_| {
        let (c2, st2) = (c.clone(), st.clone());
        widgets::ask_text(&c, "Nova lista", "", "Ex.: Trabalho, Casa, Estudos", move |name| {
            if let Some(id) = c2.check(c2.db.insert_task_list(&name, 1003)) {
                st2.sel.set(Sel::List(id));
                c2.changed();
            }
        });
    });

    let (c, st) = (ctx.clone(), state.clone());
    manage.connect_clicked(move |_| {
        let Sel::List(id) = st.sel.get() else { return };
        let lists = c.check(c.db.task_lists()).unwrap_or_default();
        if let Some(l) = lists.iter().find(|l| l.id == id) {
            list_actions(&c, id, &l.name);
        }
    });

    render(ctx, &state, &ui);
    let (c, st, u) = (ctx.clone(), state.clone(), ui.clone());
    ctx.on_refresh(move || render(&c, &st, &u));
    page
}

/// Linha da coluna de listas: ícone (ou bolinha de cor), nome e contador.
fn side_row(icon: Option<&str>, dot_color: Option<i64>, name: &str, count: Option<usize>) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    if let Some(icon) = icon {
        b.append(&gtk::Image::from_icon_name(icon));
    }
    if let Some(code) = dot_color {
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("list-dot");
        dot.add_css_class(&format!("habit-color-{}", color(code).code));
        dot.set_valign(gtk::Align::Center);
        dot.set_halign(gtk::Align::Center);
        b.append(&dot);
    }
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&label);
    if let Some(n) = count.filter(|n| *n > 0) {
        let count = gtk::Label::new(Some(&n.to_string()));
        count.add_css_class("sidebar-count");
        b.append(&count);
    }
    gtk::ListBoxRow::builder().child(&b).build()
}

fn side_heading(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("sidebar-heading");
    gtk::ListBoxRow::builder().child(&label).activatable(false).selectable(false).build()
}

/// Reconstrói a coluna de listas e o conteúdo. `false` quando a página já foi fechada.
fn render(ctx: &Shared, state: &State, ui: &Ui) -> bool {
    let (Some(side), Some(content), Some(title), Some(manage)) =
        (ui.side.upgrade(), ui.content.upgrade(), ui.title.upgrade(), ui.manage.upgrade())
    else {
        return false;
    };
    let all = ctx.check(ctx.db.tasks()).unwrap_or_default();
    let lists = ctx.check(ctx.db.task_lists()).unwrap_or_default();
    let today = checkin::today();
    let now = unloop_core::now_ms();

    // Lista apagada (aqui ou pela CLI): volta para a Caixa de entrada.
    if let Sel::List(id) = state.sel.get() {
        if !lists.iter().any(|l| l.id == id) {
            state.sel.set(Sel::View(TaskView::Inbox));
        }
    }
    let sel = state.sel.get();

    // Coluna.
    side.remove_all();
    let mut rows = Vec::new();
    for view in VIEWS {
        let count = (view != TaskView::Done).then(|| tasks::filter_view(&all, view, today, now).len());
        side.append(&side_row(Some(view_icon(view)), None, view.label(), count));
        rows.push(Some(Sel::View(view)));
    }
    if !lists.is_empty() {
        side.append(&side_heading("Listas"));
        rows.push(None);
        for l in &lists {
            let count = all.iter().filter(|t| !t.is_done() && t.list_id == Some(l.id)).count();
            side.append(&side_row(None, Some(l.color), &l.name, Some(count)));
            rows.push(Some(Sel::List(l.id)));
        }
    }
    if let Some(i) = rows.iter().position(|r| *r == Some(sel)) {
        side.select_row(side.row_at_index(i as i32).as_ref());
    }
    *state.rows.borrow_mut() = rows;

    // Conteúdo.
    widgets::clear_box(&content);
    match sel {
        Sel::View(view) => {
            let open = if view == TaskView::Done { 0 } else { tasks::filter_view(&all, view, today, now).len() };
            title.set_title(view.label());
            title.set_subtitle(&count_text(open));
            manage.set_visible(false);
            view_content(ctx, view, &content);
        }
        Sel::List(id) => {
            let name = lists.iter().find(|l| l.id == id).map_or("", |l| l.name.as_str());
            let mut open: Vec<&Task> = all.iter().filter(|t| !t.is_done() && t.list_id == Some(id)).collect();
            tasks::sort_tasks(&mut open);
            let mut done: Vec<&Task> = all.iter().filter(|t| t.is_done() && t.list_id == Some(id)).collect();
            done.sort_by(|a, b| b.done_at.cmp(&a.done_at));
            title.set_title(name);
            title.set_subtitle(&count_text(open.len()));
            manage.set_visible(true);

            let add_group = adw::PreferencesGroup::new();
            add_group.add(&quick_add(ctx, false, Some(id)));
            content.append(&add_group);
            section(ctx, &content, "Abertas", &open, &lists, false, false);
            let shown: Vec<&Task> = done.into_iter().take(20).collect();
            section(ctx, &content, "Concluídas", &shown, &lists, false, false);
            if open.is_empty() {
                empty(&content, "Lista vazia", "Adicione uma tarefa no campo acima.");
            }
        }
    }
    true
}

fn count_text(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "1 tarefa".to_string(),
        n => format!("{n} tarefas"),
    }
}

fn list_name(lists: &[TaskList], id: Option<i64>) -> String {
    id.and_then(|i| lists.iter().find(|l| l.id == i)).map_or_else(|| "Caixa de entrada".to_string(), |l| l.name.clone())
}

fn task_subtitle(t: &Task, lists: &[TaskList], today: checkin::NaiveDate, now: i64, show_list: bool) -> String {
    let mut parts = Vec::new();
    if let Some(label) = tasks::due_label(t, today) {
        parts.push(if tasks::is_overdue(t, today, now) { format!("⚠️ {label}") } else { format!("📅 {label}") });
    }
    if show_list {
        parts.push(list_name(lists, t.list_id));
    }
    let (done, total) = t.subtask_progress();
    if total > 0 {
        parts.push(format!("☑ {done}/{total}"));
    }
    if !t.tags.is_empty() {
        parts.push(t.tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" "));
    }
    if !t.description.is_empty() {
        parts.push("📝".into());
    }
    parts.join(" · ")
}

/// Linha de tarefa com caixa de concluir.
pub fn task_row(ctx: &Shared, t: &Task, lists: &[TaskList], show_list: bool) -> adw::ActionRow {
    let today = checkin::today();
    let now = unloop_core::now_ms();
    let title = format!("{}{}{}", t.priority.emoji(), if t.priority == Priority::Unset { "" } else { " " }, t.title);
    let row = adw::ActionRow::builder()
        .title(esc(&title))
        .subtitle(esc(&task_subtitle(t, lists, today, now, show_list)))
        .activatable(true)
        .build();
    if t.is_done() {
        row.add_css_class("dim-label");
    }
    let check = gtk::CheckButton::new();
    check.set_active(t.is_done());
    check.set_valign(gtk::Align::Center);
    let (c, id) = (ctx.clone(), t.id);
    check.connect_toggled(move |cb| {
        if c.check(c.db.set_task_done(id, cb.is_active())).is_some() {
            if cb.is_active() {
                c.toast("Tarefa concluída ✓");
            }
            changed_later(&c);
        }
    });
    row.add_prefix(&check);
    let (c, id) = (ctx.clone(), t.id);
    row.connect_activated(move |_| {
        if let Some(task) = c.check(c.db.task(id)).flatten() {
            c.nav.push(&editor(&c, Some(task)));
        }
    });
    row
}

/// Campo de adição rápida. `due_today`: prazo hoje; `list_id`: lista da tarefa.
pub fn quick_add(ctx: &Shared, due_today: bool, list_id: Option<i64>) -> adw::EntryRow {
    let entry = adw::EntryRow::builder()
        .title("Adicionar tarefa (Enter para salvar)")
        .show_apply_button(true)
        .build();
    let c = ctx.clone();
    entry.connect_apply(move |row| {
        let title = row.text().trim().to_string();
        if title.is_empty() {
            return;
        }
        let mut task = Task::new(&title);
        task.list_id = list_id;
        if due_today {
            task.due_at = Some(local_ms(checkin::today(), 12, 0));
        }
        if c.check(c.db.insert_task(&task)).is_some() {
            row.set_text("");
            changed_later(&c);
        }
    });
    entry
}

fn view_content(ctx: &Shared, view: TaskView, b: &gtk::Box) {
    let all = ctx.check(ctx.db.tasks()).unwrap_or_default();
    let lists = ctx.check(ctx.db.task_lists()).unwrap_or_default();
    let today = checkin::today();
    let now = unloop_core::now_ms();

    if view != TaskView::Done {
        let add_group = adw::PreferencesGroup::new();
        add_group.add(&quick_add(ctx, matches!(view, TaskView::Today | TaskView::Next7), None));
        b.append(&add_group);
    }

    match view {
        TaskView::Today => {
            let overdue = tasks::filter_view(&all, TaskView::Overdue, today, now);
            let today_tasks: Vec<&Task> = tasks::filter_view(&all, TaskView::Today, today, now)
                .into_iter()
                .filter(|t| !tasks::is_overdue(t, today, now))
                .collect();
            section(ctx, b, "⚠️ Atrasadas", &overdue, &lists, true, false);
            section(ctx, b, "Hoje", &today_tasks, &lists, true, false);
            if overdue.is_empty() && today_tasks.is_empty() {
                empty(b, "Nada para hoje", "Aproveite — ou adicione uma tarefa acima.");
            }
        }
        TaskView::Next7 => {
            let week = tasks::filter_view(&all, TaskView::Next7, today, now);
            let mut day = today;
            for _ in 0..7 {
                let items: Vec<&Task> = week.iter().copied().filter(|t| tasks::due_day(t) == Some(day)).collect();
                let title = match (day - today).num_days() {
                    0 => "Hoje".to_string(),
                    1 => "Amanhã".to_string(),
                    _ => format!(
                        "{} {}",
                        checkin::WEEKDAY_SHORT[checkin::Datelike::weekday(&day).num_days_from_monday() as usize],
                        day.format("%d/%m")
                    ),
                };
                section(ctx, b, &title, &items, &lists, true, false);
                match day.succ_opt() {
                    Some(next) => day = next,
                    None => break,
                }
            }
            if week.is_empty() {
                empty(b, "Semana livre", "Nenhuma tarefa com prazo nos próximos 7 dias.");
            }
        }
        TaskView::Inbox => {
            let items = tasks::filter_view(&all, TaskView::Inbox, today, now);
            section(ctx, b, "Caixa de entrada", &items, &lists, false, false);
            if items.is_empty() {
                empty(b, "Caixa de entrada vazia", "Tarefas sem lista aparecem aqui.");
            }
        }
        TaskView::All => {
            let open = tasks::filter_view(&all, TaskView::All, today, now);
            let inbox: Vec<&Task> = open.iter().copied().filter(|t| t.list_id.is_none()).collect();
            section(ctx, b, "Caixa de entrada", &inbox, &lists, false, false);
            for l in &lists {
                let items: Vec<&Task> = open.iter().copied().filter(|t| t.list_id == Some(l.id)).collect();
                let group = section(ctx, b, &l.name, &items, &lists, false, true);
                let manage = widgets::icon_button("view-more-symbolic", "Renomear ou apagar a lista");
                let (c, id, name) = (ctx.clone(), l.id, l.name.clone());
                manage.connect_clicked(move |_| list_actions(&c, id, &name));
                group.set_header_suffix(Some(&manage));
                if items.is_empty() {
                    group.set_description(Some("Sem tarefas abertas."));
                }
            }
            if open.is_empty() && lists.is_empty() {
                empty(b, "Nenhuma tarefa", "Crie uma com o campo acima ou o botão +.");
            }
        }
        TaskView::Done => {
            let done = tasks::filter_view(&all, TaskView::Done, today, now);
            let shown: Vec<&Task> = done.into_iter().take(100).collect();
            section(ctx, b, "Concluídas (últimas 100)", &shown, &lists, true, false);
            if shown.is_empty() {
                empty(b, "Nada concluído ainda", "As tarefas concluídas aparecem aqui.");
            }
        }
        TaskView::Overdue => {}
    }
}

/// Grupo com as tarefas. Vazio só aparece com `always` (ex.: listas, para poder gerenciá-las).
fn section(
    ctx: &Shared,
    b: &gtk::Box,
    title: &str,
    items: &[&Task],
    lists: &[TaskList],
    show_list: bool,
    always: bool,
) -> adw::PreferencesGroup {
    let group = widgets::group(title, None);
    if !items.is_empty() {
        let list = widgets::boxed_list();
        for t in items {
            list.append(&task_row(ctx, t, lists, show_list));
        }
        group.add(&list);
    }
    if !items.is_empty() || always {
        b.append(&group);
    }
    group
}

fn empty(b: &gtk::Box, title: &str, description: &str) {
    let status = adw::StatusPage::builder()
        .icon_name("object-select-symbolic")
        .title(title)
        .description(description)
        .build();
    b.append(&status);
}

/// Renomear ou apagar uma lista (as tarefas dela vão para a Caixa de entrada).
fn list_actions(ctx: &Shared, id: i64, name: &str) {
    let entry = gtk::Entry::builder().text(name).activates_default(true).width_chars(30).build();
    let dialog = adw::MessageDialog::new(
        Some(&ctx.window),
        Some("Lista"),
        Some("Renomeie ou apague a lista. Ao apagar, as tarefas vão para a Caixa de entrada."),
    );
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Cancelar"), ("delete", "Apagar lista"), ("rename", "Renomear")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");
    let c = ctx.clone();
    dialog.connect_response(None, move |_, response| match response {
        "rename" => {
            let new_name = entry.text().trim().to_string();
            if !new_name.is_empty() && c.check(c.db.rename_task_list(id, &new_name)).is_some() {
                c.changed();
            }
        }
        "delete" => {
            if c.check(c.db.delete_task_list(id)).is_some() {
                c.changed();
            }
        }
        _ => {}
    });
    dialog.present();
}

// ------------------------------------------------------------------ Editor

pub fn editor(ctx: &Shared, task: Option<Task>) -> adw::NavigationPage {
    // Tarefa com id 0 = nova, já preenchida (ex.: lista ou prazo da visão aberta).
    let editing = task.as_ref().is_some_and(|t| t.id != 0);
    let original = task.unwrap_or_else(|| Task::new(""));
    let lists = ctx.check(ctx.db.task_lists()).unwrap_or_default();
    let habits = ctx.check(ctx.db.habits()).unwrap_or_default();
    let due = Rc::new(Cell::new(original.due_at));
    let prefs = adw::PreferencesPage::new();

    // Principal
    let main = widgets::group("Tarefa", None);
    let title = adw::EntryRow::builder().title("Título").build();
    title.set_text(&original.title);
    main.add(&title);

    let mut list_labels = vec!["📥 Caixa de entrada".to_string()];
    list_labels.extend(lists.iter().map(|l| format!("📁 {}", l.name)));
    let refs: Vec<&str> = list_labels.iter().map(String::as_str).collect();
    let list_row = adw::ComboRow::builder().title("Lista").model(&gtk::StringList::new(&refs)).build();
    list_row.set_selected(
        original.list_id.and_then(|id| lists.iter().position(|l| l.id == id)).map_or(0, |i| i + 1) as u32,
    );
    main.add(&list_row);

    let priorities: Vec<String> =
        Priority::ALL.iter().map(|p| format!("{} {}", p.emoji(), p.label()).trim().to_string()).collect();
    let refs: Vec<&str> = priorities.iter().map(String::as_str).collect();
    let priority_row = adw::ComboRow::builder().title("Prioridade").model(&gtk::StringList::new(&refs)).build();
    priority_row.set_selected(original.priority.index() as u32);
    main.add(&priority_row);

    let tags = adw::EntryRow::builder().title("Tags (separadas por vírgula)").build();
    tags.set_text(&join_tags(&original.tags));
    main.add(&tags);
    prefs.add(&main);

    // Prazo
    let when = widgets::group("Prazo", None);
    let due_row = adw::ActionRow::builder().title("Data").build();
    let has_time = adw::SwitchRow::builder().title("Com horário").active(original.due_has_time).build();
    let describe = {
        let (row, switch) = (due_row.downgrade(), has_time.downgrade());
        move |value: Option<i64>| {
            if let (Some(r), Some(s)) = (row.upgrade(), switch.upgrade()) {
                r.set_subtitle(&match value {
                    Some(ms) if s.is_active() => widgets::fmt_datetime(ms),
                    Some(ms) => widgets::fmt_date(ms),
                    None => "Sem prazo".to_string(),
                });
                s.set_sensitive(value.is_some());
            }
        }
    };
    describe(original.due_at);
    let pick = gtk::Button::with_label("Escolher");
    pick.set_valign(gtk::Align::Center);
    let clear = widgets::icon_button("edit-clear-symbolic", "Remover prazo");
    due_row.add_suffix(&pick);
    due_row.add_suffix(&clear);
    let describe = Rc::new(describe);
    {
        let (c, d, desc) = (ctx.clone(), due.clone(), describe.clone());
        pick.connect_clicked(move |_| {
            let (d, desc) = (d.clone(), desc.clone());
            let initial = d.get().unwrap_or_else(|| local_ms(checkin::today(), 9, 0));
            widgets::pick_datetime(&c, "Prazo", initial, None, None, move |ms| {
                d.set(Some(ms));
                (*desc)(Some(ms));
            });
        });
    }
    {
        let (d, desc) = (due.clone(), describe.clone());
        clear.connect_clicked(move |_| {
            d.set(None);
            (*desc)(None);
        });
    }
    {
        let (d, desc) = (due.clone(), describe.clone());
        has_time.connect_active_notify(move |_| (*desc)(d.get()));
    }
    when.add(&due_row);
    when.add(&has_time);
    prefs.add(&when);

    // Descrição
    let desc_group = widgets::group("Descrição", Some("Detalhes, contexto, links… (Markdown simples)."));
    let description = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    description.buffer().set_text(&original.description);
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&description));
    frame.set_height_request(200);
    frame.add_css_class("view");
    desc_group.add(&frame);
    prefs.add(&desc_group);

    // Hábito relacionado
    let link = widgets::group("Relacionar", None);
    let mut habit_labels = vec!["Nenhum hábito".to_string()];
    habit_labels.extend(habits.iter().map(|h| format!("{}  {}", h.icon_info().emoji, h.name)));
    let refs: Vec<&str> = habit_labels.iter().map(String::as_str).collect();
    let habit_row = adw::ComboRow::builder().title("Hábito").model(&gtk::StringList::new(&refs)).build();
    habit_row.set_selected(
        original.habit_id.and_then(|id| habits.iter().position(|h| h.id == id)).map_or(0, |i| i + 1) as u32,
    );
    link.add(&habit_row);
    prefs.add(&link);

    // Subtarefas (só depois de salvar)
    if editing {
        let subs = widgets::group("Subtarefas", None);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
        subs.add(&container);
        render_subtasks(ctx, original.id, &container);
        prefs.add(&subs);

        let actions = widgets::group("Ações", None);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_halign(gtk::Align::Center);
        let done = widgets::pill_button(if original.is_done() { "Reabrir" } else { "Concluir" }, Some("suggested-action"));
        let focus_button = widgets::pill_button("🍅 Foco nesta tarefa", None);
        let delete = widgets::pill_button("Apagar", Some("destructive-action"));
        buttons.append(&done);
        buttons.append(&focus_button);
        buttons.append(&delete);
        actions.add(&buttons);
        prefs.add(&actions);

        let (c, id, is_done) = (ctx.clone(), original.id, original.is_done());
        done.connect_clicked(move |_| {
            if c.check(c.db.set_task_done(id, !is_done)).is_some() {
                c.changed();
                c.nav.pop();
            }
        });
        let (c, id) = (ctx.clone(), original.id);
        focus_button.connect_clicked(move |_| {
            c.timer.borrow_mut().task_id = Some(id);
            crate::shell::go(&c, crate::shell::Section::Focus);
        });
        let (c, id) = (ctx.clone(), original.id);
        delete.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::confirm(&c, "Apagar tarefa?", "A tarefa e as subtarefas serão apagadas.", "Apagar", true, move || {
                if c2.check(c2.db.delete_task(id)).is_some() {
                    c2.nav.pop();
                    c2.changed();
                }
            });
        });
    }

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar tarefa" } else { "Nova tarefa" })
        .child(&toolbar)
        .build();

    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let name = title.text().trim().to_string();
        if name.is_empty() {
            c.toast("Dê um título à tarefa.");
            return;
        }
        let buffer = description.buffer();
        let (start, end) = buffer.bounds();
        let mut t = original.clone();
        t.title = name;
        t.description = buffer.text(&start, &end, false).trim().to_string();
        t.list_id = (list_row.selected() as usize).checked_sub(1).and_then(|i| lists.get(i)).map(|l| l.id);
        t.priority = Priority::from_index(priority_row.selected() as usize);
        t.tags = parse_tags(&tags.text());
        t.habit_id = (habit_row.selected() as usize).checked_sub(1).and_then(|i| habits.get(i)).map(|h| h.id);
        let timed = has_time.is_active() && due.get().is_some();
        t.due_has_time = timed;
        t.due_at = due.get().map(|ms| if timed { ms } else { local_ms(local_date(ms), 12, 0) });
        let ok = if editing { c.check(c.db.update_task(&t)).is_some() } else { c.check(c.db.insert_task(&t)).is_some() };
        if ok {
            c.changed();
            c.nav.pop();
        }
    });
    page
}

fn render_subtasks(ctx: &Shared, task_id: i64, container: &gtk::Box) {
    widgets::clear_box(container);
    let subtasks = ctx.check(ctx.db.task(task_id)).flatten().map(|t| t.subtasks).unwrap_or_default();
    let list = widgets::boxed_list();
    let rerender = {
        let (c, w) = (ctx.clone(), container.downgrade());
        move || {
            let (c, w) = (c.clone(), w.clone());
            gtk::glib::idle_add_local_once(move || {
                if let Some(b) = w.upgrade() {
                    render_subtasks(&c, task_id, &b);
                }
                c.changed();
            });
        }
    };
    let rerender = Rc::new(rerender);
    for s in subtasks {
        let row = adw::ActionRow::builder().title(esc(&s.title)).build();
        let check = gtk::CheckButton::new();
        check.set_active(s.done);
        check.set_valign(gtk::Align::Center);
        let (c, id, r) = (ctx.clone(), s.id, rerender.clone());
        check.connect_toggled(move |cb| {
            if c.check(c.db.set_subtask_done(id, cb.is_active())).is_some() {
                (*r)();
            }
        });
        row.add_prefix(&check);
        let delete = widgets::icon_button("user-trash-symbolic", "Apagar subtarefa");
        let (c, id, r) = (ctx.clone(), s.id, rerender.clone());
        delete.connect_clicked(move |_| {
            if c.check(c.db.delete_subtask(id)).is_some() {
                (*r)();
            }
        });
        row.add_suffix(&delete);
        list.append(&row);
    }
    let add = adw::EntryRow::builder().title("Nova subtarefa (Enter)").show_apply_button(true).build();
    let (c, r) = (ctx.clone(), rerender.clone());
    add.connect_apply(move |row| {
        let title = row.text().trim().to_string();
        if !title.is_empty() && c.check(c.db.add_subtask(task_id, &title)).is_some() {
            (*r)();
        }
    });
    list.append(&add);
    container.append(&list);
}
