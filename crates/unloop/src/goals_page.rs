//! Metas em cascata, como no second brain: Ano → Semestre → Trimestre → Mês → Semana.
//!
//! - "Agora": as metas dos períodos de hoje, do macro (ano) ao micro (semana).
//! - "Árvore": todas as metas do ano, cada uma dentro da sua meta-mãe.
//! - Detalhe: caminho até o ano, metas filhas, tarefas e hábitos ligados.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use unloop_core::checkin::{self, Datelike, NaiveDate};
use unloop_core::model::{Habit, Task};
use unloop_core::objectives::{self as obj, Level, Objective, Status};

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};

/// Tudo o que as telas de metas leem do banco.
struct Data {
    all: Vec<Objective>,
    tasks: Vec<Task>,
    habits: Vec<Habit>,
}

fn load(ctx: &Shared) -> Data {
    Data {
        all: ctx.check(ctx.db.objectives()).unwrap_or_default(),
        tasks: ctx.check(ctx.db.tasks()).unwrap_or_default(),
        habits: ctx.check(ctx.db.habits()).unwrap_or_default(),
    }
}

fn level_icon(level: Level) -> &'static str {
    match level {
        Level::Year => "🗓️",
        Level::Semester => "🧭",
        Level::Quarter => "🏔️",
        Level::Month => "📅",
        Level::Week => "🎯",
    }
}

/// Título com o estado: riscada fica tachada, concluída com ✓.
fn title_markup(o: &Objective) -> String {
    match o.status {
        Status::Dropped => format!("<s>{}</s>", esc(&o.title)),
        Status::Done => format!("✓ {}", esc(&o.title)),
        Status::Open => esc(&o.title),
    }
}

fn progress_widget(fraction: f64, percent: u32, dim: bool) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.set_valign(gtk::Align::Center);
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(fraction);
    bar.set_valign(gtk::Align::Center);
    bar.set_width_request(80);
    let label = gtk::Label::new(Some(&format!("{percent}%")));
    label.add_css_class("caption");
    label.add_css_class("numeric");
    label.set_width_chars(4);
    label.set_xalign(1.0);
    if dim {
        b.add_css_class("dim-label");
    }
    b.append(&bar);
    b.append(&label);
    b
}

/// Linha de meta: título, mãe/área/filhas no subtítulo e barra de progresso.
fn goal_row(ctx: &Shared, o: &Objective, data: &Data, show_period: bool) -> adw::ActionRow {
    let p = obj::progress(o, &data.all, &data.tasks);
    let mut sub = Vec::new();
    if show_period {
        sub.push(o.period_label());
    }
    if let Some(parent) = o.parent_id.and_then(|id| data.all.iter().find(|x| x.id == id)) {
        sub.push(format!("↑ {}", parent.title));
    }
    if !o.area.is_empty() {
        sub.push(o.area.clone());
    }
    let kids = obj::children(&data.all, o.id);
    if !kids.is_empty() {
        sub.push(format!("{}/{} {}", p.done, p.total, kids[0].level.label().to_lowercase()));
    } else if data.tasks.iter().any(|t| t.objective_id == Some(o.id)) {
        sub.push(format!("☑ {}/{}", p.done, p.total));
    }
    if !o.status_note.is_empty() {
        sub.push(o.status_note.clone());
    }
    let row = adw::ActionRow::builder()
        .title(title_markup(o))
        .subtitle(esc(&sub.join(" · ")))
        .activatable(true)
        .build();
    if o.status != Status::Open {
        row.add_css_class("dim-label");
    }
    if o.status != Status::Dropped {
        row.add_suffix(&progress_widget(p.fraction(), p.percent(), o.status == Status::Done));
    }
    let (c, id) = (ctx.clone(), o.id);
    row.connect_activated(move |_| c.nav.push(&detail(&c, id)));
    row
}

// ---------------------------------------------------------------- Seção

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let today = checkin::today();
    let chain = obj::current_chain(today);
    let subtitle = chain.iter().map(|(l, k)| obj::short_label(*l, k)).collect::<Vec<_>>().join(" › ");
    let header = crate::shell::header("Metas", &subtitle);
    let add = widgets::icon_button("list-add-symbolic", "Nova meta");
    header.pack_start(&add);
    let c = ctx.clone();
    add.connect_clicked(move |_| c.nav.push(&editor(&c, None, Defaults::default())));

    let stack = adw::ViewStack::new();
    let now_box = widgets::content_box();
    let tree_box = widgets::content_box();
    stack.add_titled_with_icon(&widgets::scroller(&now_box), Some("now"), "Agora", "weather-clear-symbolic");
    stack.add_titled_with_icon(&widgets::scroller(&tree_box), Some("tree"), "Árvore", "view-list-symbolic");
    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    header.set_title_widget(Some(&switcher));

    let year = Rc::new(Cell::new(today.year()));
    fill_now(ctx, &now_box);
    fill_tree(ctx, &tree_box, &year);
    let (c, wn, wt, y) = (ctx.clone(), now_box.downgrade(), tree_box.downgrade(), year.clone());
    ctx.on_refresh(move || match (wn.upgrade(), wt.upgrade()) {
        (Some(n), Some(t)) => {
            widgets::clear_box(&n);
            fill_now(&c, &n);
            widgets::clear_box(&t);
            fill_tree(&c, &t, &y);
            true
        }
        _ => false,
    });

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    adw::NavigationPage::builder().title("Metas").child(&toolbar).build()
}

/// A cascata de hoje: um grupo por nível, do ano à semana.
fn fill_now(ctx: &Shared, b: &gtk::Box) {
    let data = load(ctx);
    let today = checkin::today();
    b.append(&widgets::label(
        "Do macro ao micro: a meta do ano vira metas do semestre, do trimestre, do mês e da semana. \
         Cada meta mostra a meta-mãe e o progresso sobe pela cascata.",
        &["dim-label"],
    ));
    for (level, period) in obj::current_chain(today) {
        let here: Vec<&Objective> = data.all.iter().filter(|o| o.level == level && o.period == period).collect();
        let title = format!("{} {} · {}", level_icon(level), level.label(), obj::period_label(level, &period));
        let open = here.iter().filter(|o| o.status == Status::Open).count();
        let description = if here.is_empty() {
            "Nenhuma meta neste período.".to_string()
        } else {
            format!("{} meta(s) · {open} em andamento", here.len())
        };
        let group = widgets::group(&title, Some(&description));
        let add = widgets::icon_button("list-add-symbolic", &format!("Nova meta de {}", level.label().to_lowercase()));
        add.add_css_class("flat");
        let (c, p) = (ctx.clone(), period.clone());
        add.connect_clicked(move |_| {
            c.nav.push(&editor(&c, None, Defaults { level: Some(level), period: Some(p.clone()), parent: None }))
        });
        group.set_header_suffix(Some(&add));
        if !here.is_empty() {
            let list = widgets::boxed_list();
            for o in here {
                list.append(&goal_row(ctx, o, &data, false));
            }
            group.add(&list);
        }
        b.append(&group);
    }
}

/// Árvore do ano: metas-raiz e as filhas aninhadas.
fn fill_tree(ctx: &Shared, b: &gtk::Box, year: &Rc<Cell<i32>>) {
    let data = load(ctx);
    let y = year.get();

    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    nav.set_halign(gtk::Align::Center);
    let prev = widgets::icon_button("go-previous-symbolic", "Ano anterior");
    let next = widgets::icon_button("go-next-symbolic", "Próximo ano");
    let label = gtk::Label::new(Some(&y.to_string()));
    label.add_css_class("title-2");
    nav.append(&prev);
    nav.append(&label);
    nav.append(&next);
    for (button, delta) in [(prev, -1), (next, 1)] {
        let (c, yr) = (ctx.clone(), year.clone());
        button.connect_clicked(move |_| {
            yr.set(yr.get() + delta);
            changed_later(&c);
        });
    }
    b.append(&nav);

    let Some((start, end)) = obj::period_range(Level::Year, &y.to_string()) else { return };
    let in_year = |o: &Objective| obj::anchor(o.level, &o.period).is_some_and(|a| a >= start && a <= end);
    let roots: Vec<&Objective> = data
        .all
        .iter()
        .filter(|o| o.parent_id.is_none_or(|p| !data.all.iter().any(|x| x.id == p)) && in_year(o))
        .collect();
    if roots.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("emoji-flags-symbolic")
            .title(format!("Nenhuma meta em {y}"))
            .description("Comece pela meta do ano e desça: semestre, trimestre, mês e semana.")
            .build();
        b.append(&status);
        return;
    }
    let today = checkin::today();
    let chain = obj::current_chain(today);
    let list = widgets::boxed_list();
    for r in roots {
        list.append(&tree_row(ctx, r, &data, &chain, 0));
    }
    b.append(&list);
}

fn tree_row(ctx: &Shared, o: &Objective, data: &Data, chain: &[(Level, String)], depth: usize) -> gtk::Widget {
    let kids = obj::children(&data.all, o.id);
    if kids.is_empty() || depth > 6 {
        let row = goal_row(ctx, o, data, true);
        row.set_title(&format!("{} {}", level_icon(o.level), title_markup(o)));
        return row.upcast();
    }
    let p = obj::progress(o, &data.all, &data.tasks);
    let mut sub = vec![o.period_label()];
    if !o.area.is_empty() {
        sub.push(o.area.clone());
    }
    sub.push(format!("{}/{} {}", p.done, p.total, kids[0].level.label().to_lowercase()));
    let row = adw::ExpanderRow::builder()
        .title(format!("{} {}", level_icon(o.level), title_markup(o)))
        .subtitle(esc(&sub.join(" · ")))
        .build();
    // Aberta quando está no caminho de hoje (o período atual do nível).
    row.set_expanded(chain.iter().any(|(l, k)| *l == o.level && *k == o.period) && o.status == Status::Open);
    if o.status != Status::Dropped {
        row.add_suffix(&progress_widget(p.fraction(), p.percent(), o.status == Status::Done));
    }
    let open = widgets::icon_button("go-next-symbolic", "Abrir a meta");
    open.add_css_class("flat");
    let (c, id) = (ctx.clone(), o.id);
    open.connect_clicked(move |_| c.nav.push(&detail(&c, id)));
    row.add_suffix(&open);
    for k in kids {
        row.add_row(&tree_row(ctx, k, data, chain, depth + 1));
    }
    row.upcast()
}

// --------------------------------------------------------------- Detalhe

pub fn detail(ctx: &Shared, id: i64) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    let edit = widgets::icon_button("document-edit-symbolic", "Editar");
    header.pack_end(&edit);
    let c = ctx.clone();
    edit.connect_clicked(move |_| {
        if let Some(o) = c.check(c.db.objective(id)).flatten() {
            c.nav.push(&editor(&c, Some(o), Defaults::default()));
        }
    });
    let content = widgets::content_box();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&widgets::scroller(&content)));
    let page = adw::NavigationPage::builder().title("Meta").child(&toolbar).build();

    let fill = {
        let c = ctx.clone();
        let wp = page.downgrade();
        move |b: &gtk::Box| -> bool {
            let data = load(&c);
            let Some(o) = data.all.iter().find(|x| x.id == id).cloned() else { return false };
            if let Some(p) = wp.upgrade() {
                p.set_title(&format!("{} · {}", o.level.label(), obj::short_label(o.level, &o.period)));
            }
            fill_detail(&c, b, &o, &data);
            true
        }
    };
    if fill(&content) {
        let weak = content.downgrade();
        ctx.on_refresh(move || match weak.upgrade() {
            Some(b) => {
                widgets::clear_box(&b);
                // A meta foi apagada (pela CLI ou por outra tela): a página fica vazia.
                fill(&b)
            }
            None => false,
        });
    }
    page
}

fn fill_detail(ctx: &Shared, b: &gtk::Box, o: &Objective, data: &Data) {
    let p = obj::progress(o, &data.all, &data.tasks);

    // Caminho do ano até aqui.
    let path = obj::ancestry(&data.all, o.id);
    if path.len() > 1 {
        let crumbs = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(10).build();
        for (i, a) in path.iter().enumerate() {
            let text = format!("{}{} {}", if i > 0 { "› " } else { "" }, obj::short_label(a.level, &a.period), a.title);
            let button = gtk::Button::with_label(&text);
            button.add_css_class("flat");
            button.add_css_class("caption");
            if a.id == o.id {
                button.set_sensitive(false);
            } else {
                let (c, aid) = (ctx.clone(), a.id);
                button.connect_clicked(move |_| c.nav.push(&detail(&c, aid)));
            }
            crumbs.append(&button);
        }
        b.append(&crumbs);
    }

    let title = gtk::Label::new(None);
    title.set_markup(&format!("<span size='x-large' weight='bold'>{}</span>", title_markup(o)));
    title.set_wrap(true);
    title.set_xalign(0.0);
    b.append(&title);
    let mut meta = vec![format!("{} {} · {}", level_icon(o.level), o.level.label(), o.period_label())];
    meta.push(o.status.label().to_string());
    if !o.area.is_empty() {
        meta.push(o.area.clone());
    }
    let meta_label = widgets::label(&meta.join(" · "), &["dim-label"]);
    meta_label.set_xalign(0.0);
    b.append(&meta_label);
    if !o.status_note.is_empty() {
        let note = widgets::label(&format!("“{}”", o.status_note), &["dim-label"]);
        note.set_xalign(0.0);
        b.append(&note);
    }
    if o.status != Status::Dropped {
        let bar = gtk::ProgressBar::new();
        bar.set_fraction(p.fraction());
        bar.add_css_class("thick");
        bar.set_show_text(true);
        bar.set_text(Some(&format!("{}% · {}/{}", p.percent(), p.done, p.total)));
        b.append(&bar);
    }
    if !o.description.is_empty() {
        let desc = widgets::label(&o.description, &[]);
        desc.set_xalign(0.0);
        desc.set_selectable(true);
        b.append(&desc);
    }

    // Ações.
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    let id = o.id;
    if o.status == Status::Open {
        let done = widgets::pill_button("Concluir", Some("suggested-action"));
        let c = ctx.clone();
        done.connect_clicked(move |_| {
            if c.check(c.db.set_objective_status(id, Status::Done, "")).is_some() {
                c.toast("Meta concluída ✓");
                changed_later(&c);
            }
        });
        buttons.append(&done);
        let drop = widgets::pill_button("Riscar", None);
        drop.set_tooltip_text(Some("Não rodou, foi remanejada ou saiu do plano. Deixa de contar no progresso da meta-mãe."));
        let c = ctx.clone();
        drop.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::ask_text(&c, "Riscar a meta", "", "Motivo e destino (ex.: remanejado pra W48)", move |note| {
                if c2.check(c2.db.set_objective_status(id, Status::Dropped, &note)).is_some() {
                    changed_later(&c2);
                }
            });
        });
        buttons.append(&drop);
    } else {
        let reopen = widgets::pill_button("Reabrir", None);
        let c = ctx.clone();
        reopen.connect_clicked(move |_| {
            if c.check(c.db.set_objective_status(id, Status::Open, "")).is_some() {
                changed_later(&c);
            }
        });
        buttons.append(&reopen);
    }
    let delete = widgets::pill_button("Apagar", Some("destructive-action"));
    let below = obj::descendants(&data.all, o.id).len();
    let c = ctx.clone();
    delete.connect_clicked(move |_| {
        let c2 = c.clone();
        let body = if below > 0 {
            format!("Apaga também as {below} meta(s) abaixo dela. As tarefas ligadas ficam sem meta.")
        } else {
            "As tarefas ligadas ficam sem meta.".to_string()
        };
        widgets::confirm(&c, "Apagar meta?", &body, "Apagar", true, move || {
            if c2.check(c2.db.delete_objective(id)).is_some() {
                c2.nav.pop();
                c2.changed();
            }
        });
    });
    buttons.append(&delete);
    b.append(&buttons);

    // Metas filhas.
    let kids = obj::children(&data.all, o.id);
    if let Some(child_level) = o.level.child() {
        let title = format!("Metas abaixo ({})", child_level.label().to_lowercase());
        let group = widgets::group(&title, if kids.is_empty() { Some("Quebre esta meta no nível de baixo.") } else { None });
        let add = widgets::icon_button("list-add-symbolic", &format!("Nova meta de {}", child_level.label().to_lowercase()));
        add.add_css_class("flat");
        let c = ctx.clone();
        add.connect_clicked(move |_| {
            c.nav.push(&editor(&c, None, Defaults { level: Some(child_level), period: None, parent: Some(id) }))
        });
        group.set_header_suffix(Some(&add));
        if !kids.is_empty() {
            let list = widgets::boxed_list();
            for k in kids {
                list.append(&goal_row(ctx, k, data, true));
            }
            group.add(&list);
        }
        b.append(&group);
    }

    // Tarefas ligadas.
    let lists = ctx.check(ctx.db.task_lists()).unwrap_or_default();
    let tasks: Vec<&Task> = data.tasks.iter().filter(|t| t.objective_id == Some(o.id)).collect();
    let group = widgets::group("Tarefas", Some("O que fazer para cumprir a meta."));
    let list = widgets::boxed_list();
    let entry = adw::EntryRow::builder().title("Adicionar tarefa (Enter para salvar)").show_apply_button(true).build();
    let c = ctx.clone();
    entry.connect_apply(move |row| {
        let title = row.text().trim().to_string();
        if title.is_empty() {
            return;
        }
        let mut task = Task::new(&title);
        task.objective_id = Some(id);
        if c.check(c.db.insert_task(&task)).is_some() {
            row.set_text("");
            changed_later(&c);
        }
    });
    list.append(&entry);
    for t in tasks {
        list.append(&crate::tasks_page::task_row(ctx, t, &lists, true));
    }
    group.add(&list);
    b.append(&group);

    // Hábitos ligados.
    let linked: Vec<&Habit> = o.habit_ids.iter().filter_map(|hid| data.habits.iter().find(|h| h.id == *hid)).collect();
    if !linked.is_empty() {
        let group = widgets::group("Hábitos", Some("Os hábitos que sustentam esta meta."));
        let list = widgets::boxed_list();
        for h in linked {
            let row = adw::ActionRow::builder().title(esc(&h.name)).activatable(true).build();
            row.add_prefix(&widgets::avatar(h.icon, h.color, false));
            let (c, hid, build) = (ctx.clone(), h.id, h.is_build());
            row.connect_activated(move |_| {
                c.nav.push(&if build { crate::checkin_page::page(&c, hid) } else { crate::detail::page(&c, hid) })
            });
            list.append(&row);
        }
        group.add(&list);
        b.append(&group);
    }
}

// ---------------------------------------------------------------- Editor

/// Valores iniciais de uma meta nova.
#[derive(Clone, Default)]
pub struct Defaults {
    pub level: Option<Level>,
    pub period: Option<String>,
    pub parent: Option<i64>,
}

/// Períodos de um nível para escolher: do ano passado ao próximo (e o atual da meta, se fora).
fn period_options(level: Level, today: NaiveDate, keep: Option<&str>) -> Vec<String> {
    let from = NaiveDate::from_ymd_opt(today.year() - 1, 1, 1).unwrap_or(today);
    let to = NaiveDate::from_ymd_opt(today.year() + 1, 12, 31).unwrap_or(today);
    let mut out = obj::periods_between(level, from, to);
    if let Some(k) = keep {
        if !out.iter().any(|x| x == k) && obj::period_range(level, k).is_some() {
            out.push(k.to_string());
            out.sort();
        }
    }
    out
}

pub fn editor(ctx: &Shared, existing: Option<Objective>, defaults: Defaults) -> adw::NavigationPage {
    let today = checkin::today();
    let editing = existing.is_some();
    let all = ctx.check(ctx.db.objectives()).unwrap_or_default();
    let habits = ctx.check(ctx.db.habits()).unwrap_or_default();
    let parent_default = defaults.parent.and_then(|id| all.iter().find(|o| o.id == id)).cloned();
    let original = existing.unwrap_or_else(|| {
        let level = defaults
            .level
            .or_else(|| parent_default.as_ref().and_then(|p| p.level.child()))
            .unwrap_or(Level::Year);
        let period = defaults.period.clone().unwrap_or_else(|| {
            let key = obj::period_key(level, today);
            match &parent_default {
                Some(p) if !obj::contains(p.level, &p.period, level, &key) => p
                    .range()
                    .and_then(|(s, e)| obj::periods_between(level, s, e).into_iter().next())
                    .unwrap_or(key),
                _ => key,
            }
        });
        let mut o = Objective::new(level, &period, "");
        o.parent_id = parent_default.as_ref().map(|p| p.id);
        o.area = parent_default.as_ref().map(|p| p.area.clone()).unwrap_or_default();
        o
    });
    // A meta e as descendentes não podem ser mãe dela.
    let blocked: Vec<i64> = std::iter::once(original.id)
        .chain(obj::descendants(&all, original.id).iter().map(|d| d.id))
        .filter(|id| *id != 0)
        .collect();

    let prefs = adw::PreferencesPage::new();
    let main = widgets::group("Meta", None);
    let title = adw::EntryRow::builder().title("Título").build();
    title.set_text(&original.title);
    main.add(&title);
    let area = adw::EntryRow::builder().title("Área (ex.: Estudo, Financeiro)").build();
    area.set_text(&original.area);
    main.add(&area);
    prefs.add(&main);

    let when = widgets::group("Quando", Some("O nível e o período da meta. A meta-mãe precisa ser de um nível acima e conter o período."));
    let level_labels: Vec<String> = Level::ALL.iter().map(|l| format!("{} {}", level_icon(*l), l.label())).collect();
    let refs: Vec<&str> = level_labels.iter().map(String::as_str).collect();
    let level_row = adw::ComboRow::builder().title("Nível").model(&gtk::StringList::new(&refs)).build();
    level_row.set_selected(original.level as u32);
    let period_row = adw::ComboRow::builder().title("Período").build();
    let parent_row = adw::ComboRow::builder().title("Meta-mãe").build();
    when.add(&level_row);
    when.add(&period_row);
    when.add(&parent_row);
    prefs.add(&when);

    // Opções que mudam com o nível e o período.
    let periods: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let parents: Rc<std::cell::RefCell<Vec<i64>>> = Rc::default();
    let refresh_parents = {
        let (all, parents, periods, blocked) = (all.clone(), parents.clone(), periods.clone(), blocked.clone());
        let (pr, lr, par) = (period_row.downgrade(), level_row.downgrade(), parent_row.downgrade());
        move |keep: Option<i64>| {
            let (Some(pr), Some(lr), Some(par)) = (pr.upgrade(), lr.upgrade(), par.upgrade()) else { return };
            let level = Level::ALL[lr.selected() as usize];
            let Some(period) = periods.borrow().get(pr.selected() as usize).cloned() else { return };
            let current = keep.or_else(|| parents.borrow().get((par.selected() as usize).wrapping_sub(1)).copied());
            let options: Vec<&Objective> = all
                .iter()
                .filter(|o| !blocked.contains(&o.id) && obj::contains(o.level, &o.period, level, &period))
                .collect();
            let mut labels = vec!["Sem meta-mãe".to_string()];
            labels.extend(options.iter().map(|o| format!("[{}] {}", obj::short_label(o.level, &o.period), o.title)));
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            par.set_model(Some(&gtk::StringList::new(&refs)));
            *parents.borrow_mut() = options.iter().map(|o| o.id).collect();
            let index = current.and_then(|id| options.iter().position(|o| o.id == id)).map_or(0, |i| i + 1);
            par.set_selected(index as u32);
        }
    };
    let refresh_periods = {
        let periods = periods.clone();
        let (pr, lr) = (period_row.downgrade(), level_row.downgrade());
        move |keep: Option<String>| {
            let (Some(pr), Some(lr)) = (pr.upgrade(), lr.upgrade()) else { return };
            let level = Level::ALL[lr.selected() as usize];
            let options = period_options(level, today, keep.as_deref());
            let labels: Vec<String> = options.iter().map(|k| obj::period_label(level, k)).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let wanted = keep.unwrap_or_else(|| obj::period_key(level, today));
            let index = options.iter().position(|k| *k == wanted).unwrap_or(0);
            // A lista precisa estar atualizada antes de trocar o modelo (que avisa a mudança de seleção).
            *periods.borrow_mut() = options;
            pr.set_model(Some(&gtk::StringList::new(&refs)));
            pr.set_selected(index as u32);
        }
    };
    let refresh_parents = Rc::new(refresh_parents);
    let building = Rc::new(Cell::new(true));
    refresh_periods(Some(original.period.clone()));
    refresh_parents(original.parent_id);
    {
        let (rp, rpa, b) = (refresh_periods.clone(), refresh_parents.clone(), building.clone());
        level_row.connect_selected_notify(move |_| {
            if !b.get() {
                rp(None);
                rpa(None);
            }
        });
    }
    {
        let (rpa, b) = (refresh_parents.clone(), building.clone());
        period_row.connect_selected_notify(move |_| {
            if !b.get() {
                rpa(None);
            }
        });
    }
    building.set(false);

    // Descrição.
    let desc_group = widgets::group("Descrição", Some("Contexto, números, links para o plano (Markdown simples)."));
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
    frame.set_height_request(140);
    frame.add_css_class("view");
    desc_group.add(&frame);
    prefs.add(&desc_group);

    // Hábitos.
    let checks: Vec<(i64, gtk::CheckButton)> = habits
        .iter()
        .map(|h| {
            let check = gtk::CheckButton::new();
            check.set_active(original.habit_ids.contains(&h.id));
            (h.id, check)
        })
        .collect();
    if !habits.is_empty() {
        let group = widgets::group("Hábitos ligados", Some("Os hábitos que sustentam esta meta."));
        for (h, (_, check)) in habits.iter().zip(&checks) {
            let row = adw::ActionRow::builder().title(esc(&h.name)).activatable_widget(check).build();
            row.add_prefix(&widgets::avatar(h.icon, h.color, false));
            row.add_suffix(check);
            group.add(&row);
        }
        prefs.add(&group);
    }

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder().title(if editing { "Editar meta" } else { "Nova meta" }).child(&toolbar).build();

    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let name = title.text().trim().to_string();
        if name.is_empty() {
            c.toast("Dê um título à meta.");
            return;
        }
        let level = Level::ALL[level_row.selected() as usize];
        let Some(period) = periods.borrow().get(period_row.selected() as usize).cloned() else {
            c.toast("Escolha o período.");
            return;
        };
        let parent_id = parents.borrow().get((parent_row.selected() as usize).wrapping_sub(1)).copied();
        // As filhas precisam continuar cabendo no novo período.
        let mut o = original.clone();
        o.level = level;
        o.period = period;
        if editing {
            for k in obj::children(&all, o.id) {
                if let Err(e) = obj::check_parent(k.level, &k.period, &o) {
                    c.toast(&format!("A meta \"{}\" deixaria de caber: {e}", k.title));
                    return;
                }
            }
        }
        let buffer = description.buffer();
        let (start, end) = buffer.bounds();
        o.title = name;
        o.area = area.text().trim().to_string();
        o.description = buffer.text(&start, &end, false).trim().to_string();
        o.parent_id = parent_id;
        o.habit_ids = checks.iter().filter(|(_, ch)| ch.is_active()).map(|(id, _)| *id).collect();
        let ok = if editing {
            c.check(c.db.update_objective(&o)).is_some()
        } else {
            c.check(c.db.insert_objective(&o)).is_some()
        };
        if ok {
            c.changed();
            c.nav.pop();
        }
    });
    page
}

/// Grupo "Metas da semana" da tela Hoje.
pub fn week_group(ctx: &Shared) -> Option<adw::PreferencesGroup> {
    let data = load(ctx);
    let today = checkin::today();
    let week = obj::period_key(Level::Week, today);
    let month = obj::period_key(Level::Month, today);
    let here: Vec<&Objective> = data
        .all
        .iter()
        .filter(|o| o.status == Status::Open && ((o.level == Level::Week && o.period == week) || (o.level == Level::Month && o.period == month)))
        .collect();
    if data.all.is_empty() {
        return None;
    }
    let description = if here.is_empty() {
        "Nenhuma meta em andamento na semana nem no mês.".to_string()
    } else {
        format!("{} · {}", obj::period_label(Level::Week, &week), obj::period_label(Level::Month, &month))
    };
    let group = widgets::group("Metas da semana e do mês", Some(&description));
    if !here.is_empty() {
        let list = widgets::boxed_list();
        // Semana primeiro (o micro, o que se faz agora), depois o mês.
        let mut sorted = here;
        sorted.sort_by_key(|o| std::cmp::Reverse(o.level));
        for o in sorted {
            list.append(&goal_row(ctx, o, &data, true));
        }
        group.add(&list);
    }
    Some(group)
}
